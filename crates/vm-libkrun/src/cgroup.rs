use crate::{
    config::{CgroupLimits, LibkrunConfig},
    validation::PROVIDER_NAME,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use vm_trait::VmError;

pub struct Cgroup {
    path: PathBuf,
    emulated: bool,
}

impl Cgroup {
    pub fn create(config: &LibkrunConfig, id: &str) -> Result<Self, VmError> {
        let path = config.cgroup_root.join(id);
        fs::create_dir(&path).map_err(|error| provider("cgroup-create", error))?;
        let cgroup = Self {
            path,
            emulated: !config.enforce_cgroup_v2,
        };
        if let Err(error) = cgroup.configure(&config.limits) {
            let _cleanup_result = fs::remove_dir(&cgroup.path);
            return Err(error);
        }
        Ok(cgroup)
    }

    pub fn add_process(&self, pid: u32) -> Result<(), VmError> {
        write(self.path.join("cgroup.procs"), pid.to_string())
            .map_err(|error| provider("cgroup-place-worker", error))
    }

    pub fn cleanup(&self) -> Result<(), VmError> {
        if self.emulated {
            return remove_directory_tree(&self.path);
        }
        let kill = self.path.join("cgroup.kill");
        if kill.exists() {
            write(kill, "1").map_err(|error| provider("cgroup-kill", error))?;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.is_populated()? && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        match fs::remove_dir(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(provider("cgroup-cleanup", error)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn configure(&self, limits: &CgroupLimits) -> Result<(), VmError> {
        let cpu = limits.cpu_quota_micros.map_or_else(
            || format!("max {}", limits.cpu_period_micros),
            |quota| format!("{quota} {}", limits.cpu_period_micros),
        );
        write(self.path.join("cpu.max"), cpu)
            .map_err(|error| provider("cgroup-cpu-limit", error))?;
        write(
            self.path.join("memory.max"),
            limits.memory_max_bytes.to_string(),
        )
        .map_err(|error| provider("cgroup-memory-limit", error))?;
        write(self.path.join("pids.max"), limits.pids_max.to_string())
            .map_err(|error| provider("cgroup-pids-limit", error))?;

        if !limits.io.is_empty() {
            let mut lines = String::new();
            for limit in &limits.io {
                use std::fmt::Write as _;
                write!(&mut lines, "{}:{}", limit.major, limit.minor)
                    .map_err(|error| provider("cgroup-io-format", io::Error::other(error)))?;
                if let Some(value) = limit.read_bps {
                    write!(&mut lines, " rbps={value}")
                        .map_err(|error| provider("cgroup-io-format", io::Error::other(error)))?;
                }
                if let Some(value) = limit.write_bps {
                    write!(&mut lines, " wbps={value}")
                        .map_err(|error| provider("cgroup-io-format", io::Error::other(error)))?;
                }
                if let Some(value) = limit.read_iops {
                    write!(&mut lines, " riops={value}")
                        .map_err(|error| provider("cgroup-io-format", io::Error::other(error)))?;
                }
                if let Some(value) = limit.write_iops {
                    write!(&mut lines, " wiops={value}")
                        .map_err(|error| provider("cgroup-io-format", io::Error::other(error)))?;
                }
                lines.push('\n');
            }
            write(self.path.join("io.max"), lines)
                .map_err(|error| provider("cgroup-io-limit", error))?;
        }
        Ok(())
    }

    fn is_populated(&self) -> Result<bool, VmError> {
        let events = self.path.join("cgroup.events");
        if !events.exists() {
            return Ok(false);
        }
        let events =
            fs::read_to_string(events).map_err(|error| provider("cgroup-events", error))?;
        Ok(events
            .lines()
            .any(|line| line.split_whitespace().eq(["populated", "1"])))
    }
}

fn remove_directory_tree(path: &Path) -> Result<(), VmError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(provider("cgroup-cleanup", error)),
    }
}

fn write(path: PathBuf, value: impl AsRef<[u8]>) -> io::Result<()> {
    fs::write(path, value)
}

fn provider(code: &'static str, source: io::Error) -> VmError {
    VmError::Provider {
        provider: PROVIDER_NAME.to_owned(),
        code: code.to_owned(),
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::Cgroup;
    use crate::config::{CgroupLimits, LibkrunConfig};
    use std::{fs, os::unix::fs::PermissionsExt};
    use tempfile::TempDir;

    #[test]
    fn cgroup_writes_all_configured_limits() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("cgroup");
        fs::create_dir(&root).unwrap();
        for file in ["cpu.max", "memory.max", "pids.max", "io.max"] {
            fs::write(root.join(file), "").unwrap();
        }

        let executable = temp.path().join("executable");
        fs::write(&executable, "").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = LibkrunConfig::new(
            temp.path(),
            vec![temp.path().to_path_buf()],
            vec![temp.path().to_path_buf()],
            vec![temp.path().to_path_buf()],
            &executable,
            &root,
        );
        config.enforce_cgroup_v2 = false;
        config.limits = CgroupLimits::default();

        let vm = root.join("vm");
        fs::create_dir(&vm).unwrap();
        for file in ["cpu.max", "memory.max", "pids.max"] {
            fs::write(vm.join(file), "").unwrap();
        }
        let cgroup = Cgroup {
            path: vm.clone(),
            emulated: true,
        };
        cgroup.configure(&config.limits).unwrap();
        assert!(
            fs::read_to_string(vm.join("cpu.max"))
                .unwrap()
                .starts_with("max ")
        );
        assert!(
            !fs::read_to_string(vm.join("memory.max"))
                .unwrap()
                .is_empty()
        );
    }
}

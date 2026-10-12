use crate::{
    config::{CgroupLimits, LibkrunConfig},
    validation::PROVIDER_NAME,
};
use std::{
    collections::HashSet,
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
    pub(crate) fn create_tracked(
        config: &LibkrunConfig,
        id: &str,
        acquired: impl FnOnce(&Self),
    ) -> Result<Self, VmError> {
        let path = config.cgroup_root.join(id);
        fs::create_dir(&path).map_err(|error| provider("cgroup-create", error))?;
        let cgroup = Self {
            path,
            emulated: !config.enforce_cgroup_v2,
        };
        // Publish exclusive mkdir ownership before configuration can fail.
        acquired(&cgroup);
        cgroup.configure(&config.limits)?;
        Ok(cgroup)
    }

    pub fn existing(config: &LibkrunConfig, id: &str) -> Self {
        Self {
            path: config.cgroup_root.join(id),
            emulated: !config.enforce_cgroup_v2,
        }
    }

    pub fn add_process(&self, pid: u32) -> Result<(), VmError> {
        write(self.path.join("cgroup.procs"), pid.to_string())
            .map_err(|error| provider("cgroup-place-worker", error))
    }

    pub fn cleanup(&self) -> Result<(), VmError> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(provider(
                    "cgroup-cleanup",
                    io::Error::other("cgroup path is not a directory"),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return confirm_absent(&self.path);
            }
            Err(error) => return Err(provider("cgroup-cleanup", error)),
        }
        if self.emulated {
            return remove_directory_tree(&self.path);
        }
        let kill = self.path.join("cgroup.kill");
        match fs::symlink_metadata(&kill) {
            Ok(metadata) if metadata.is_file() => {
                write(kill, "1").map_err(|error| provider("cgroup-kill", error))?;
            }
            Ok(_) => {
                return Err(provider(
                    "cgroup-kill",
                    io::Error::other("cgroup.kill is not a regular control file"),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(provider("cgroup-kill", error)),
        }
        self.wait_until_empty(Instant::now() + Duration::from_secs(2))?;
        match fs::remove_dir(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => confirm_absent(&self.path),
            Err(error) => Err(provider("cgroup-cleanup", error)),
        }
    }

    fn wait_until_empty(&self, deadline: Instant) -> Result<(), VmError> {
        loop {
            if !self.is_populated()? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(provider(
                    "cgroup-cleanup",
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        "cgroup is still populated at cleanup deadline",
                    ),
                ));
            }
            thread::sleep(Duration::from_millis(10));
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
        let path = self.path.join("cgroup.events");
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| provider("cgroup-events", error))?;
        if !metadata.is_file() {
            return Err(provider(
                "cgroup-events",
                io::Error::other("cgroup.events is not a regular control file"),
            ));
        }
        let events = fs::read_to_string(path).map_err(|error| provider("cgroup-events", error))?;
        parse_populated(&events).map_err(|error| provider("cgroup-events", error))
    }
}

// cgroup.events is flat-keyed. Require one exact populated 0/1 observation;
// malformed or duplicate fields cannot silently become an empty-cgroup proof.
fn parse_populated(events: &str) -> io::Result<bool> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "malformed cgroup.events populated observation",
        )
    };
    let mut keys = HashSet::new();
    let mut populated = None;
    for line in events.lines() {
        let mut fields = line.split_whitespace();
        let key = fields.next().ok_or_else(invalid)?;
        let value = fields.next().ok_or_else(invalid)?;
        if fields.next().is_some()
            || !keys.insert(key)
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(invalid());
        }
        if key == "populated" {
            populated = Some(match value {
                "0" => false,
                "1" => true,
                _ => return Err(invalid()),
            });
        }
    }
    populated.ok_or_else(invalid)
}

// Preserve exact known-child idempotence, but a missing or unreadable parent
// cannot establish that this child was removed. Never infer a root-wide drain.
fn confirm_absent(path: &Path) -> Result<(), VmError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(provider("cgroup-cleanup", error)),
        Ok(_) => {
            return Err(provider(
                "cgroup-cleanup",
                io::Error::other("cgroup path is still present"),
            ));
        }
    }
    let parent = path.parent().ok_or_else(|| {
        provider(
            "cgroup-cleanup",
            io::Error::other("cgroup path has no parent"),
        )
    })?;
    let metadata = fs::metadata(parent).map_err(|error| provider("cgroup-cleanup", error))?;
    if !metadata.is_dir() {
        return Err(provider(
            "cgroup-cleanup",
            io::Error::other("cgroup parent is not a directory"),
        ));
    }
    Ok(())
}

fn remove_directory_tree(path: &Path) -> Result<(), VmError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => confirm_absent(path),
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

#[cfg(test)]
#[path = "cgroup/cleanup_tests.rs"]
mod cleanup_tests;

//! Existing limit configuration and populated control observation.
use super::{Cgroup, parse_populated, provider, write};
use crate::config::CgroupLimits;
use std::{
    fs,
    io::{self, Read, Write},
};
use vm_trait::VmError;

impl Cgroup {
    pub(super) fn control_file(
        &self,
        name: &str,
        flags: rustix::fs::OFlags,
    ) -> io::Result<fs::File> {
        let child = self
            .child
            .as_ref()
            .ok_or_else(|| io::Error::other("cgroup child is not pinned"))?;
        let directory = child.file()?;
        let file: fs::File = rustix::fs::openat(
            &*directory,
            name,
            flags
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::from_bits_truncate(0o600),
        )?
        .into();
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("cgroup control is not a regular file"));
        }
        Ok(file)
    }

    pub(super) fn write_control(&self, name: &str, value: impl AsRef<[u8]>) -> io::Result<()> {
        if self.child.is_none() {
            return write(self.path.join(name), value);
        }
        let mut flags = rustix::fs::OFlags::WRONLY | rustix::fs::OFlags::TRUNC;
        if self.emulated {
            flags |= rustix::fs::OFlags::CREATE;
        }
        self.control_file(name, flags)?.write_all(value.as_ref())
    }

    pub(super) fn configure(&self, limits: &CgroupLimits) -> Result<(), VmError> {
        let cpu = limits.cpu_quota_micros.map_or_else(
            || format!("max {}", limits.cpu_period_micros),
            |quota| format!("{quota} {}", limits.cpu_period_micros),
        );
        self.write_control("cpu.max", cpu)
            .map_err(|error| provider("cgroup-cpu-limit", error))?;
        self.write_control("memory.max", limits.memory_max_bytes.to_string())
            .map_err(|error| provider("cgroup-memory-limit", error))?;
        self.write_control("pids.max", limits.pids_max.to_string())
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
            self.write_control("io.max", lines)
                .map_err(|error| provider("cgroup-io-limit", error))?;
        }
        Ok(())
    }

    pub(super) fn is_populated(&self) -> Result<bool, VmError> {
        if self.child.is_some() {
            let mut events = String::new();
            self.control_file("cgroup.events", rustix::fs::OFlags::RDONLY)
                .and_then(|mut file| file.read_to_string(&mut events))
                .map_err(|error| provider("cgroup-events", error))?;
            return parse_populated(&events).map_err(|error| provider("cgroup-events", error));
        }
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

//! Durable owner metadata for explicitly managed libkrun roots.

mod configured;
#[cfg(test)]
mod configured_provider_tests;
#[cfg(test)]
mod configured_tests;
mod files;
#[cfg(test)]
mod supervision_tests;
#[cfg(test)]
mod tests;

use crate::config::LibkrunConfig;
use serde::{Deserialize, Serialize};
use std::{fs::File, os::unix::fs::MetadataExt};
use vm_trait::{VmError, VmProviderOwnerScope};

use files::{locked_root, read_marker, write_marker};

const MARKER: &str = ".heph-vm-owner.json";
const LOCK: &str = ".heph-vm-owner.lock";
const SUPERVISOR: &str = ".heph-vm-supervisor.lock";

pub(super) fn reject_unowned(config: &LibkrunConfig) -> Result<(), VmError> {
    files::reject_marked_root(config)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
    uid: u32,
}

impl FileIdentity {
    fn read(file: &File) -> Result<Self, VmError> {
        let metadata = file.metadata().map_err(files::io_error)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
    uid: u32,
}

impl DirectoryIdentity {
    fn read(file: &File) -> Result<Self, VmError> {
        let metadata = file.metadata().map_err(files::io_error)?;
        if !metadata.is_dir() {
            return Err(files::invalid("owner root is not a directory"));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u32,
    namespace: uuid::Uuid,
    host_id: String,
    runtime: DirectoryIdentity,
    cgroup: DirectoryIdentity,
    supervisor: FileIdentity,
}

pub(super) struct ProviderOwner {
    marker: Marker,
    // This exact open file description owns the lifetime exclusive flock.
    // Never clone it for workers or convert it to the shared physical IO lock.
    supervisor: File,
    // Explicit configured owners keep their original directory descriptors for
    // the entire supervisor lifetime; legacy initialization remains unchanged.
    pinned_roots: Option<PinnedCleanupRoots>,
}

/// Holds pinned directories and a shared owner lock through physical IO.
pub(super) struct OwnerGuard {
    pub runtime: File,
    pub cgroup: File,
    _lock: File,
}

/// Retains exact allocated roots for handle destruction without retaining the
/// metadata flock or changing paths sent to the worker process.
pub(super) struct PinnedCleanupRoots {
    runtime: File,
    cgroup: File,
}

impl PinnedCleanupRoots {
    pub fn from_guard(guard: &OwnerGuard) -> Result<Self, VmError> {
        Ok(Self {
            runtime: guard.runtime.try_clone().map_err(files::io_error)?,
            cgroup: guard.cgroup.try_clone().map_err(files::io_error)?,
        })
    }

    pub fn cleanup(&self, config: &LibkrunConfig, id: &vm_trait::VmId) -> Result<(), VmError> {
        use std::os::fd::AsRawFd;
        let mut pinned = config.clone();
        pinned.cgroup_root = format!("/proc/self/fd/{}", self.cgroup.as_raw_fd()).into();
        super::helpers::cleanup_runtime(
            &std::path::PathBuf::from(format!("/proc/self/fd/{}", self.runtime.as_raw_fd()))
                .join(&id.0),
        )?;
        crate::cgroup::Cgroup::existing(&pinned, &id.0).cleanup()
    }
}

impl ProviderOwner {
    pub fn initialize(config: &LibkrunConfig, host_id: &str) -> Result<Self, VmError> {
        VmProviderOwnerScope::new("validation".into(), host_id.to_owned())?;
        let existing = files::preflight_owner(config, host_id)?;
        let guard = locked_root(config, !existing)?;
        let runtime = DirectoryIdentity::read(&guard.runtime)?;
        let cgroup = DirectoryIdentity::read(&guard.cgroup)?;
        let (marker, supervisor) = if let Some(marker) = read_marker(&guard.runtime)? {
            if marker.host_id != host_id || marker.runtime != runtime || marker.cgroup != cgroup {
                return Err(files::invalid(
                    "VM owner metadata differs from configured roots/host",
                ));
            }
            let supervisor = files::supervisor_file(&guard.runtime, false, config.service_uid)?;
            if FileIdentity::read(&supervisor)? != marker.supervisor {
                return Err(files::invalid("VM supervisor lock identity changed"));
            }
            files::claim_supervisor(&supervisor)?;
            (marker, supervisor)
        } else {
            files::require_empty_runtime(&guard.runtime)?;
            files::require_unclassified_cgroups_absent(&guard.cgroup)?;
            let supervisor = files::supervisor_file(&guard.runtime, true, config.service_uid)?;
            files::claim_supervisor(&supervisor)?;
            let marker = Marker {
                version: 2,
                namespace: uuid::Uuid::new_v4(),
                host_id: host_id.to_owned(),
                runtime: runtime.clone(),
                cgroup: cgroup.clone(),
                supervisor: FileIdentity::read(&supervisor)?,
            };
            write_marker(&guard.runtime, &marker)?;
            (marker, supervisor)
        };
        if marker.version != 2
            || marker.namespace.is_nil()
            || marker.host_id != host_id
            || marker.runtime != runtime
            || marker.cgroup != cgroup
        {
            return Err(files::invalid(
                "VM owner metadata differs from configured host/root/cgroup; automatic adoption is unavailable",
            ));
        }
        Ok(Self {
            marker,
            supervisor,
            pinned_roots: None,
        })
    }

    pub fn scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        VmProviderOwnerScope::new(
            self.marker.namespace.to_string(),
            self.marker.host_id.clone(),
        )
    }

    pub fn validate(&self, config: &LibkrunConfig) -> Result<OwnerGuard, VmError> {
        // Reject redirection before opening or locking its owner metadata.
        if DirectoryIdentity::read(&files::open_root(&config.runtime_root, config.service_uid)?)?
            != self.marker.runtime
            || DirectoryIdentity::read(&files::open_root(&config.cgroup_root, config.service_uid)?)?
                != self.marker.cgroup
        {
            return Err(files::invalid("VM provider root ownership changed"));
        }
        let guard = locked_root(config, false)?;
        self.validate_guard(config, &guard)?;
        Ok(guard)
    }

    pub fn validate_guard(
        &self,
        config: &LibkrunConfig,
        guard: &OwnerGuard,
    ) -> Result<(), VmError> {
        if let Some(roots) = &self.pinned_roots {
            if DirectoryIdentity::read(&roots.runtime)? != self.marker.runtime
                || DirectoryIdentity::read(&roots.cgroup)? != self.marker.cgroup
            {
                return Err(files::invalid("pinned VM provider roots changed"));
            }
        }
        if DirectoryIdentity::read(&guard.runtime)? != self.marker.runtime
            || DirectoryIdentity::read(&guard.cgroup)? != self.marker.cgroup
            || DirectoryIdentity::read(&files::open_root(
                &config.runtime_root,
                config.service_uid,
            )?)? != self.marker.runtime
            || DirectoryIdentity::read(&files::open_root(&config.cgroup_root, config.service_uid)?)?
                != self.marker.cgroup
            || read_marker(&guard.runtime)?.as_ref() != Some(&self.marker)
        {
            return Err(files::invalid("VM provider root ownership changed"));
        }
        files::check_file(&self.supervisor, config.service_uid)?;
        let current = if self.pinned_roots.is_some() {
            configured::readonly_supervisor(&guard.runtime, config.service_uid)?
        } else {
            files::supervisor_file(&guard.runtime, false, config.service_uid)?
        };
        if FileIdentity::read(&self.supervisor)? != self.marker.supervisor
            || FileIdentity::read(&current)? != self.marker.supervisor
        {
            return Err(files::invalid("VM supervisor lock identity changed"));
        }
        Ok(())
    }
}

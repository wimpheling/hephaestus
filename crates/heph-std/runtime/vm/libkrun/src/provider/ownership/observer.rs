//! Finite original-root observation, never cleanup or admission authority.
#[path = "observer/kernel.rs"]
mod kernel;
#[cfg(test)]
#[path = "observer/native.rs"]
mod native;
#[cfg(test)]
#[path = "observer/tests.rs"]
mod tests;

use super::{LOCK, MARKER, OwnerGuard, ProviderOwner, SUPERVISOR, files};
use crate::config::LibkrunConfig;
use std::{collections::BTreeSet, ffi::OsString, fs, os::fd::AsRawFd};
use vm_trait::VmError;

impl ProviderOwner {
    pub(in crate::provider) fn assert_no_prior_births(
        &self,
        config: &LibkrunConfig,
        guard: &OwnerGuard,
    ) -> Result<(), VmError> {
        self.require_containment()?;
        if self.pinned_roots.is_none() || !config.enforce_cgroup_v2 {
            return Err(VmError::Unsupported {
                feature:
                    "absence observation requires configured pinned roots and actual cgroup v2"
                        .into(),
                provider: "libkrun".into(),
            });
        }
        self.validate_guard(config, guard)?;
        inspect_runtime(self, config, guard)?;
        let original = &self
            .pinned_roots
            .as_ref()
            .expect("configured roots were required above")
            .cgroup;
        let controls = kernel::inspect(&guard.cgroup, original)?;
        inspect_runtime(self, config, guard)?;
        kernel::recheck(&guard.cgroup, original, &controls)?;
        self.validate_guard(config, guard)
    }
}

fn runtime_entries(root: &fs::File) -> Result<BTreeSet<OsString>, VmError> {
    let mut names = BTreeSet::new();
    for entry in
        fs::read_dir(format!("/proc/self/fd/{}", root.as_raw_fd())).map_err(files::io_error)?
    {
        let name = entry.map_err(files::io_error)?.file_name();
        if ![MARKER, LOCK, SUPERVISOR]
            .map(std::ffi::OsStr::new)
            .contains(&name.as_os_str())
            || !names.insert(name)
        {
            return Err(files::invalid(
                "VM runtime contains an unknown or repeated entry",
            ));
        }
    }
    Ok(names)
}

fn inspect_runtime(
    owner: &ProviderOwner,
    config: &LibkrunConfig,
    guard: &OwnerGuard,
) -> Result<(), VmError> {
    let expected: BTreeSet<_> = [MARKER, LOCK, SUPERVISOR].map(OsString::from).into();
    if runtime_entries(&guard.runtime)? != expected {
        return Err(files::invalid(
            "VM runtime has residual or unknown birth objects",
        ));
    }
    for (name, original) in [
        (MARKER, owner.marker_file.as_ref()),
        (LOCK, owner.metadata_lock.as_ref()),
        (SUPERVISOR, &owner.supervisor),
    ] {
        // Each original remains pinned; readonly_original checks private shape,
        // nofollow named identity and link count, without acquiring any flock.
        let named = files::readonly_original(&guard.runtime, name, original, config.service_uid)?;
        drop(named);
    }
    owner.validate_guard(config, guard)
}

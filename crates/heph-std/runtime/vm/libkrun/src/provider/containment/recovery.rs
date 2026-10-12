//! Scoped physical cleanup using recorded original inodes and inherited custody.
use super::super::{helpers::provider_error, ownership::OwnerGuard};
use super::{
    birth::{BirthRecord, Identity},
    exclusive,
};
use crate::{cgroup::Cgroup, config::LibkrunConfig, directory::ChildDirectory};
use rustix::fs::{Mode, OFlags};
use std::fs::File;
use vm_trait::{VmError, VmId, VmProviderOwnerScope};

pub(in crate::provider) fn recover(
    config: &LibkrunConfig,
    guard: &OwnerGuard,
    scope: &VmProviderOwnerScope,
    id: &VmId,
) -> Result<Option<File>, VmError> {
    let Some(record) = BirthRecord::load((&guard.runtime, &guard.cgroup), scope, id)? else {
        // Only a positively qualified version3 caller reaches this boundary.
        // An unclassified named allocation is never promoted to an original.
        require_absent(&guard.runtime, id)?;
        require_absent(&guard.cgroup, id)?;
        return Ok(None);
    };
    let runtime = pin(&guard.runtime, id, &record.value.runtime)?;
    let cgroup = pin(&guard.cgroup, id, &record.value.cgroup)?;
    if runtime.is_none() && cgroup.is_some() {
        return Err(VmError::InvalidState(
            "VM runtime missing while original cgroup survives",
        ));
    }
    if let Some(cgroup) = cgroup {
        Cgroup::recorded(config, &id.0, cgroup).cleanup()?;
    }
    let mut retained_gate = None;
    if let Some(runtime) = runtime {
        let original = runtime.file().map_err(io_error)?;
        // The stopped cgroup does not contain a pre-placement worker. Its FD0
        // gate must be independently released before any original runtime IO.
        let gate = exclusive(&original)?;
        runtime.remove(true).map_err(io_error)?;
        retained_gate = Some(gate);
    }
    // A crash after child removal but before record unlink is safely retryable.
    record.remove_original()?;
    Ok(retained_gate)
}

fn pin(root: &File, id: &VmId, expected: &Identity) -> Result<Option<ChildDirectory>, VmError> {
    ChildDirectory::open_recorded(root, &id.0, expected.device, expected.inode, expected.uid)
        .map_err(io_error)
}

fn require_absent(root: &File, id: &VmId) -> Result<(), VmError> {
    match rustix::fs::openat(
        root,
        id.0.as_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err(VmError::InvalidState(
            "VM allocation lacks original containment birth record",
        )),
        Err(error) => Err(io_error(error)),
    }
}

fn io_error(error: impl std::error::Error + Send + Sync + 'static) -> VmError {
    provider_error("scoped-containment-cleanup", error)
}

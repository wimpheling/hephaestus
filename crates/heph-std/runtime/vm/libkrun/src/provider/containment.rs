//! Cooperative inherited process custody, never a global absence or reap receipt.
#[path = "containment/birth.rs"]
mod birth;
#[cfg(test)]
#[path = "containment/native.rs"]
mod native;
#[path = "containment/recovery.rs"]
mod recovery;
#[cfg(test)]
#[path = "containment/tests.rs"]
mod tests;

pub(super) use birth::BirthRecord;
pub(super) use recovery::recover;

use super::helpers::provider_error;
use rustix::fs::{FlockOperation, Mode, OFlags};
use std::{fs::File, os::unix::fs::MetadataExt, process::Stdio};
use vm_trait::VmError;

/// A separate OFD is crucial: parent directory custody must not own this gate.
/// FD0 and its child duplicates retain it; the parent only drops, never unlocks.
pub(super) fn worker_stdin(original: &File) -> Result<Stdio, VmError> {
    let gate = independent(original)?;
    rustix::fs::flock(&gate, FlockOperation::NonBlockingLockShared)
        .map_err(|error| provider_error("worker-containment-gate", error))?;
    Ok(Stdio::from(gate))
}

fn independent(original: &File) -> Result<File, VmError> {
    let file: File = rustix::fs::openat(
        original,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| provider_error("containment-directory", error))?
    .into();
    let before = original
        .metadata()
        .map_err(|error| provider_error("containment-directory", error))?;
    let after = file
        .metadata()
        .map_err(|error| provider_error("containment-directory", error))?;
    if !before.is_dir()
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.uid() != after.uid()
        || before.nlink() == 0
    {
        return Err(VmError::InvalidState(
            "containment directory identity differs",
        ));
    }
    Ok(file)
}

fn exclusive(original: &File) -> Result<File, VmError> {
    let gate = independent(original)?;
    rustix::fs::flock(&gate, FlockOperation::NonBlockingLockExclusive)
        .map_err(|error| provider_error("prior-worker-containment-held", error))?;
    Ok(gate)
}

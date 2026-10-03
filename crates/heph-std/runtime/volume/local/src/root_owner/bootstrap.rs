use super::{VolumeRootOwner, marker};
use crate::{
    backing, invalid_backing,
    owned_journal::filesystem::{self, FileIdentity},
};
use rustix::fs::{Dir, FlockOperation};
use std::{fs::File, path::Path};
use volume_trait::{VolumeError, VolumeRootNamespaceId};

impl VolumeRootOwner {
    /// Publishes a configured namespace in an existing empty prospective root.
    ///
    /// The caller must supply trusted durable configuration and independently
    /// check immutable root history. This filesystem operation grants no actor
    /// permission and proves neither database absence nor file adoption.
    /// Existing owners must use [`Self::open_existing`] instead. No directory,
    /// random namespace, replacement or repair is created here.
    ///
    /// # Errors
    /// Rejects unsafe, noncanonical, nonempty, interrupted or contended roots,
    /// invalid host labels and replacement during pinned publication. Interrupted
    /// publication evidence remains held rather than being removed or repaired.
    pub fn bootstrap_empty_with_namespace(
        path: &Path,
        host: &str,
        namespace: VolumeRootNamespaceId,
    ) -> Result<Self, VolumeError> {
        bootstrap(path, host, namespace, || {})
    }
}

fn bootstrap(
    path: &Path,
    host: &str,
    namespace: VolumeRootNamespaceId,
    before_publication: impl FnOnce(),
) -> Result<VolumeRootOwner, VolumeError> {
    if host.is_empty() || host.len() > 128 || host.chars().any(char::is_control) {
        return Err(VolumeError::IntentConflict);
    }
    let root = marker::open_root(path)?;
    let identity = FileIdentity::of(&root).map_err(backing)?;
    // openat(".") has an independent open file description. A dup/try_clone of
    // the returned owner's descriptor could retain this transient flock.
    let publication = filesystem::open_directory(&root, ".").map_err(backing)?;
    if FileIdentity::of(&publication).map_err(backing)? != identity {
        return Err(invalid_backing("bootstrap root descriptor differs"));
    }
    rustix::fs::flock(&publication, FlockOperation::NonBlockingLockExclusive).map_err(backing)?;
    only_entry(&root, None)?;
    before_publication();
    checked_root(&root, path, identity)?;
    only_entry(&root, None)?;
    let (owner_marker, bytes, actual_namespace) =
        marker::publish(&root, host, identity, namespace, || {
            checked_root(&root, path, identity)?;
            only_entry(&root, Some(marker::PENDING))
        })?;
    let result = VolumeRootOwner {
        root,
        marker: owner_marker,
        path: path.into(),
        host: host.into(),
        namespace: actual_namespace,
        identity,
        bytes,
    };
    result.validate()?;
    only_entry(&result.root, Some(marker::NAME))?;
    // This separate descriptor closes on every success/error exit. Serialization
    // covers cooperating new bootstraps, not the legacy initializer or arbitrary
    // same-UID writers, and is never a lifetime supervisor/ownership authority.
    drop(publication);
    Ok(result)
}

fn checked_root(root: &File, path: &Path, identity: FileIdentity) -> Result<(), VolumeError> {
    filesystem::validate_directory(root, false).map_err(backing)?;
    if FileIdentity::of(root).map_err(backing)? != identity
        || FileIdentity::of(&marker::open_root(path)?).map_err(backing)? != identity
    {
        return Err(invalid_backing("root changed during empty bootstrap"));
    }
    Ok(())
}

fn only_entry(root: &File, expected: Option<&str>) -> Result<(), VolumeError> {
    let mut directory = Dir::read_from(root).map_err(backing)?;
    let mut found = false;
    while let Some(entry) = directory.read() {
        let entry = entry.map_err(backing)?;
        let name = entry.file_name().to_bytes();
        if matches!(name, b"." | b"..") {
            continue;
        }
        if expected.is_none_or(|allowed| name != allowed.as_bytes()) || found {
            return Err(invalid_backing("prospective volume root is not empty"));
        }
        found = true;
    }
    if expected.is_some() && !found {
        return Err(invalid_backing("bootstrap publication record disappeared"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;

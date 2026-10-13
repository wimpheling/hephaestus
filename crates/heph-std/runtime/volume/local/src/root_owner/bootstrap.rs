use super::{VolumeRootOwner, marker};
use crate::{
    backing, invalid_backing,
    owned_journal::filesystem::{self, FileIdentity},
};
use rustix::fs::{Dir, FlockOperation};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
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

/// Pinned, readonly preparation for an explicitly configured empty root.
///
/// Retains the original nofollow directory descriptor and an independent
/// cooperative publication flock. There are no marker/pending writes until
/// [`Self::publish`] consumes this handle. Drop releases only descriptors/locks;
/// it grants no adoption, bootstrap authorization, database fence or cleanup.
/// This lock excludes cooperating preparations, not legacy initializers or
/// arbitrary same-UID writers. Two roots remain separate publications.
#[must_use = "preparation must be consumed to publish, or dropped without writes"]
pub struct ProspectiveVolumeRootBootstrap {
    root: File,
    publication: File,
    path: PathBuf,
    host: String,
    namespace: VolumeRootNamespaceId,
    identity: FileIdentity,
}

impl VolumeRootOwner {
    /// Pins and validates an existing canonical safe empty root without writes.
    ///
    /// Trusted configuration supplies prospective intent and the explicit owner
    /// namespace. The caller separately observes immutable history. No root,
    /// marker, pending record or random identity is created. Existing owners
    /// must instead use [`Self::open_existing`].
    ///
    /// # Errors
    /// Rejects invalid host/configuration, unsafe, noncanonical, nonempty or
    /// contended roots, and root replacement during preparation.
    pub fn prepare_empty_with_namespace(
        path: &Path,
        host: &str,
        namespace: VolumeRootNamespaceId,
    ) -> Result<ProspectiveVolumeRootBootstrap, VolumeError> {
        if host.is_empty() || host.len() > 128 || host.chars().any(char::is_control) {
            return Err(VolumeError::IntentConflict);
        }
        let root = marker::open_root(path)?;
        let identity = FileIdentity::of(&root).map_err(backing)?;
        // Independent open file description keeps this transient flock out of
        // the eventual owner's retained root descriptor.
        let publication = filesystem::open_directory(&root, ".").map_err(backing)?;
        if FileIdentity::of(&publication).map_err(backing)? != identity {
            return Err(invalid_backing("bootstrap root descriptor differs"));
        }
        rustix::fs::flock(&publication, FlockOperation::NonBlockingLockExclusive)
            .map_err(backing)?;
        checked_root(&root, path, identity)?;
        only_entry(&root, None)?;
        Ok(ProspectiveVolumeRootBootstrap {
            root,
            publication,
            path: path.into(),
            host: host.into(),
            namespace,
            identity,
        })
    }
}

impl ProspectiveVolumeRootBootstrap {
    /// Publishes this SAME pinned prospective root after rechecking it.
    ///
    /// Revalidates original descriptor/path dev/inode and emptiness before
    /// reservation and again before marker bytes. No rebootstrap or adoption
    /// occurs. Failure after reservation preserves original pending evidence;
    /// no marker or pending record is removed or rolled back.
    ///
    /// # Errors
    /// Rejects changed roots/contents or failed publication/validation. Later
    /// failures can leave explicit held remnants, including a published marker.
    pub fn publish(self) -> Result<VolumeRootOwner, VolumeError> {
        let Self {
            root,
            publication,
            path,
            host,
            namespace,
            identity,
        } = self;
        checked_root(&root, &path, identity)?;
        only_entry(&root, None)?;
        let (owner_marker, bytes, actual_namespace) =
            marker::publish(&root, &host, identity, namespace, || {
                checked_root(&root, &path, identity)?;
                only_entry(&root, Some(marker::PENDING))
            })?;
        let result = VolumeRootOwner {
            root,
            marker: owner_marker,
            path,
            host,
            namespace: actual_namespace,
            identity,
            bytes,
        };
        result.validate()?;
        only_entry(&result.root, Some(marker::NAME))?;
        // Release the transient publication lock; returned ownership is not a
        // lifetime supervisor and does not retain this independent description.
        drop(publication);
        Ok(result)
    }
}

fn bootstrap(
    path: &Path,
    host: &str,
    namespace: VolumeRootNamespaceId,
    before_publication: impl FnOnce(),
) -> Result<VolumeRootOwner, VolumeError> {
    let prospective = VolumeRootOwner::prepare_empty_with_namespace(path, host, namespace)?;
    before_publication();
    prospective.publish()
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

#[cfg(test)]
#[path = "prospective_tests.rs"]
mod prospective_tests;

//! Persistent physical volume-root identity, independent of recipe and VM scopes.

mod bootstrap;
mod marker;
mod reopen;
#[cfg(test)]
mod reopen_tests;
#[cfg(test)]
mod tests;

use crate::{
    backing, invalid_backing,
    owned_journal::filesystem::{self, FileIdentity},
    owned_journal::{JournalPurpose, OwnedJournalLock},
};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use volume_trait::{VolumeError, VolumeRootNamespaceId};

/// Immutable descriptor guard binding a provider UUID to the real storage root.
///
/// This conveys no actor permission, volume ownership or authority to adopt files.
pub struct VolumeRootOwner {
    root: File,
    marker: File,
    path: PathBuf,
    host: String,
    namespace: VolumeRootNamespaceId,
    identity: FileIdentity,
    bytes: Vec<u8>,
}
impl VolumeRootOwner {
    pub(crate) fn pinned_root(&self) -> Result<(File, FileIdentity), VolumeError> {
        self.validate()?;
        Ok((self.root.try_clone().map_err(backing)?, self.identity))
    }
    pub(crate) fn journal_lock(
        &self,
        purpose: JournalPurpose,
        existing: bool,
    ) -> Result<OwnedJournalLock, VolumeError> {
        if purpose.root() != self.path {
            return Err(VolumeError::IntentConflict);
        }
        let (root, identity) = self.pinned_root()?;
        let result = OwnedJournalLock::acquire_pinned(&root, identity, purpose, existing)
            .map_err(backing)?;
        self.validate()?;
        Ok(result)
    }
    /// Opens an existing root and initializes only a prospective owner marker.
    /// Existing unsealed raw files are retained without adoption. Missing markers
    /// beside owned journals fail held rather than inventing historical ownership.
    ///
    /// # Errors
    /// Rejects root substitution, unsafe permissions, contradictory or copied markers.
    pub fn initialize(path: &Path, host: &str) -> Result<Self, VolumeError> {
        if host.is_empty() || host.len() > 128 || host.chars().any(char::is_control) {
            return Err(VolumeError::IntentConflict);
        }
        let root = marker::open_root(path)?;
        let identity = FileIdentity::of(&root).map_err(backing)?;
        let (marker, bytes, namespace) = marker::load_or_create(&root, path, host, identity)?;
        let result = Self {
            root,
            marker,
            path: path.into(),
            host: host.into(),
            namespace,
            identity,
            bytes,
        };
        result.validate()?;
        Ok(result)
    }
    /// Rechecks the actual root and marker without filesystem mutation.
    ///
    /// # Errors
    /// Fails if the root, marker, host binding or descriptor identity was replaced.
    pub fn validate(&self) -> Result<(), VolumeError> {
        filesystem::validate_directory(&self.root, false).map_err(backing)?;
        let current = marker::open_root(&self.path)?;
        let (linked, bytes) = marker::read(&current)?;
        filesystem::validate_private_file(&self.marker).map_err(backing)?;
        if FileIdentity::of(&current).map_err(backing)? != self.identity
            || FileIdentity::of(&self.root).map_err(backing)? != self.identity
            || FileIdentity::of(&linked).map_err(backing)?
                != FileIdentity::of(&self.marker).map_err(backing)?
            || bytes != self.bytes
            || filesystem::exists(&current, marker::PENDING).map_err(backing)?
        {
            return Err(invalid_backing("physical volume root owner was replaced"));
        }
        marker::decode(&bytes, self.identity, &self.host)?;
        Ok(())
    }
    /// Returns the persistent physical root namespace, distinct from VM ownership.
    #[must_use]
    pub const fn namespace_id(&self) -> VolumeRootNamespaceId {
        self.namespace
    }
    /// Returns the exact configured canonical root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.path
    }
    /// Returns the configured provider host binding.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }
}

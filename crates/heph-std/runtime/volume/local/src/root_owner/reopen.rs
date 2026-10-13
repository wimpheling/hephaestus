use super::{VolumeRootOwner, marker};
use crate::{
    backing, invalid_backing,
    owned_journal::filesystem::{self, FileIdentity},
};
use std::path::Path;
use volume_trait::{VolumeError, VolumeRootNamespaceId};

impl VolumeRootOwner {
    /// Reopens an existing physical root under its authoritative expected identity.
    ///
    /// This opens existing descriptors read-only and never creates a directory,
    /// marker or lock file. The caller must obtain the expected namespace from
    /// trusted persisted configuration or metadata; the path is not authority.
    /// An empty replacement root cannot initialize a new owner through this API.
    ///
    /// # Errors
    /// Rejects missing, pending, copied, unsafe or mismatching root/marker evidence,
    /// including another host or namespace and substitution during validation.
    pub fn open_existing(
        path: &Path,
        expected_host: &str,
        expected_namespace: VolumeRootNamespaceId,
    ) -> Result<Self, VolumeError> {
        if expected_host.is_empty()
            || expected_host.len() > 128
            || expected_host.chars().any(char::is_control)
        {
            return Err(VolumeError::IntentConflict);
        }
        let root = marker::open_root(path)?;
        let identity = FileIdentity::of(&root).map_err(backing)?;
        if filesystem::exists(&root, marker::PENDING).map_err(backing)? {
            return Err(invalid_backing(
                "incomplete root owner publication requires recovery",
            ));
        }
        let (marker, bytes) = marker::read(&root)?;
        let namespace = marker::decode(&bytes, identity, expected_host)?;
        if namespace != expected_namespace {
            return Err(invalid_backing(
                "physical volume root namespace differs from expected owner",
            ));
        }
        let result = Self {
            root,
            marker,
            path: path.into(),
            host: expected_host.into(),
            namespace,
            identity,
            bytes,
        };
        result.validate()?;
        Ok(result)
    }
}

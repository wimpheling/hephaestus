use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use volume_domain::VolumeRootNamespaceId;

use crate::{OwnedVolumeRegistrationReceipt, VolumeError};

/// Checked immutable comparison data, independent of authorization or physical proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedBackingPurpose {
    receipt: OwnedVolumeRegistrationReceipt,
    host: String,
    root: PathBuf,
    owner_namespace: VolumeRootNamespaceId,
    birth_generation: u64,
    bytes: Vec<u8>,
}
impl OwnedBackingPurpose {
    /// Checks receipt encoding, provider labels, lexical root and immutable birth fence.
    ///
    /// No host action occurs. The provider must later bind the namespace owner UUID
    /// to its actual root inode; canonical path text alone is insufficient.
    ///
    /// # Errors
    /// Rejects invalid receipt fingerprints, provenance, roots or bounded counters.
    pub fn new(
        receipt: OwnedVolumeRegistrationReceipt,
        host: String,
        root: PathBuf,
        owner_namespace: VolumeRootNamespaceId,
        birth_generation: u64,
    ) -> Result<Self, VolumeError> {
        let canonical = receipt.intent.canonical_bytes();
        let hash: [u8; 32] = Sha256::digest(&canonical).into();
        let root_text = root.to_str().ok_or(VolumeError::IntentConflict)?;
        if hash != receipt.registration_hash
            || receipt.first_request_id.is_nil()
            || receipt.event_id.is_nil()
            || receipt.event_cursor <= 0
            || receipt.event_aggregate_version <= 0
            || birth_generation == 0
            || birth_generation > i64::MAX.cast_unsigned()
            || !root.is_absolute()
            || root
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
            || !valid_text(&host, 128)
            || !valid_text(root_text, 4096)
            || root.components().collect::<PathBuf>().to_str() != Some(root_text)
        {
            return Err(VolumeError::IntentConflict);
        }
        let mut bytes = b"heph-owned-backing-purpose-v1\0".to_vec();
        append(&mut bytes, &canonical)?;
        bytes.extend_from_slice(receipt.seal_id.as_uuid().as_bytes());
        bytes.extend_from_slice(&receipt.registration_hash);
        bytes.extend_from_slice(receipt.first_request_id.as_bytes());
        bytes.extend_from_slice(receipt.event_id.as_bytes());
        bytes.extend_from_slice(&receipt.event_cursor.to_be_bytes());
        bytes.extend_from_slice(&receipt.event_aggregate_version.to_be_bytes());
        append(&mut bytes, host.as_bytes())?;
        append(&mut bytes, root_text.as_bytes())?;
        bytes.extend_from_slice(owner_namespace.as_uuid().as_bytes());
        bytes.extend_from_slice(&birth_generation.to_be_bytes());
        Ok(Self {
            receipt,
            host,
            root,
            owner_namespace,
            birth_generation,
            bytes,
        })
    }
    /// Returns the exact original metadata lineage.
    #[must_use]
    pub const fn receipt(&self) -> &OwnedVolumeRegistrationReceipt {
        &self.receipt
    }
    /// Returns the immutable configured host label.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }
    /// Returns the checked lexical root, without claiming physical owner identity.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Returns the persistent physical provider-root owner label.
    #[must_use]
    pub const fn owner_namespace(&self) -> VolumeRootNamespaceId {
        self.owner_namespace
    }
    /// Returns the original backing birth fence, independent of current CAS.
    #[must_use]
    pub const fn birth_generation(&self) -> u64 {
        self.birth_generation
    }
    /// Returns the immutable private journal namespace name.
    #[must_use]
    pub fn namespace_name(&self) -> String {
        format!(
            ".owned-{}-{}",
            self.receipt.intent.registration().id(),
            self.receipt.seal_id.as_uuid()
        )
    }
    /// Returns the canonical existing volume backing path.
    #[must_use]
    pub fn canonical_path(&self) -> PathBuf {
        self.root
            .join(format!("{}.raw", self.receipt.intent.registration().id()))
    }
    /// Returns exact versioned bytes shared with the host journal purpose.
    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Returns the SHA-256 purpose fingerprint.
    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(&self.bytes).into()
    }
}
fn valid_text(text: &str, maximum: usize) -> bool {
    !text.is_empty() && text.len() <= maximum && !text.chars().any(char::is_control)
}
fn append(bytes: &mut Vec<u8>, field: &[u8]) -> Result<(), VolumeError> {
    bytes.extend_from_slice(
        &u16::try_from(field.len())
            .map_err(|_| VolumeError::IntentConflict)?
            .to_be_bytes(),
    );
    bytes.extend_from_slice(field);
    Ok(())
}

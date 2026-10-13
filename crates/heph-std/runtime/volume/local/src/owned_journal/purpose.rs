//! Checked immutable purpose, deliberately distinct from authorization.

use super::JournalError;
use runtime_types::VolumeId;
use std::path::{Path, PathBuf};
use volume_trait::{
    OwnedBackingPurpose, OwnedVolumeRegistrationReceipt, VolumeCreationSealId,
    VolumeRootNamespaceId,
};

#[derive(Clone)]
pub struct JournalPurpose {
    id: VolumeId,
    seal: VolumeCreationSealId,
    root: PathBuf,
    generation: u64,
    capacity: u64,
    bytes: Vec<u8>,
}
impl JournalPurpose {
    /// Checks immutable comparison data. The later caller must verify the receipt
    /// against the canonical database and the authoritative original Create claim.
    pub fn new(
        receipt: &OwnedVolumeRegistrationReceipt,
        host: &str,
        root: &Path,
        owner_namespace: VolumeRootNamespaceId,
        generation: u64,
    ) -> Result<Self, JournalError> {
        let checked = OwnedBackingPurpose::new(
            receipt.clone(),
            host.to_owned(),
            root.to_path_buf(),
            owner_namespace,
            generation,
        )
        .map_err(|_| JournalError::Conflict("invalid immutable backing purpose"))?;
        let registration = receipt.intent.registration();
        let bytes = checked.canonical_bytes().to_vec();
        Ok(Self {
            id: registration.id(),
            seal: receipt.seal_id,
            root: root.to_path_buf(),
            generation,
            capacity: registration.capacity_bytes(),
            bytes,
        })
    }
    pub const fn id(&self) -> VolumeId {
        self.id
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Original backing birth generation, immutable across later reconciliation CAS attempts.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }
    pub fn canonical_name(&self) -> String {
        format!("{}.raw", self.id)
    }
    pub fn namespace_name(&self) -> String {
        format!(".owned-{}-{}", self.id, self.seal.as_uuid())
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

use std::path::{Component, Path, PathBuf};

use uuid::Uuid;

use crate::{
    OwnedBackingPurpose, OwnedProvisioningContext, OwnedVolumeRegistration,
    OwnedVolumeRegistrationReceipt, VolumeError, VolumeRootNamespaceId,
};

/// Checked backend comparison inputs; these values grant no actor or host authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedProvisioningExpectation {
    intent: OwnedVolumeRegistration,
    host: String,
    root: PathBuf,
    owner_namespace: VolumeRootNamespaceId,
}

impl OwnedProvisioningExpectation {
    /// Checks exact registration and configured physical provider labels.
    ///
    /// The composition root must supply the actual persistent root namespace;
    /// neither this constructor nor lexical path validation proves root ownership.
    ///
    /// # Errors
    /// Rejects empty, oversized, noncanonical or control-bearing provider labels.
    pub fn new(
        intent: OwnedVolumeRegistration,
        host: String,
        root: PathBuf,
        owner_namespace: VolumeRootNamespaceId,
    ) -> Result<Self, VolumeError> {
        let text = root.to_str().ok_or(VolumeError::IntentConflict)?;
        if host.is_empty()
            || host.len() > 128
            || host.chars().any(char::is_control)
            || text.is_empty()
            || text.len() > 4096
            || text.chars().any(char::is_control)
            || !root.is_absolute()
            || root
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
            || root.components().collect::<PathBuf>().to_str() != Some(text)
        {
            return Err(VolumeError::IntentConflict);
        }
        Ok(Self {
            intent,
            host,
            root,
            owner_namespace,
        })
    }

    /// Returns all immutable 110 registration and creation pins.
    #[must_use]
    pub const fn intent(&self) -> &OwnedVolumeRegistration {
        &self.intent
    }
    /// Returns the configured physical host label.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }
    /// Returns the canonical lexical provider root; not an open filesystem handle.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Returns the configured persistent physical root namespace.
    #[must_use]
    pub const fn owner_namespace(&self) -> VolumeRootNamespaceId {
        self.owner_namespace
    }
    /// Derives the version-one first-operation UUID from immutable 110 creation.
    ///
    /// This mapping allocates no fence and does not authorize admission. The
    /// original operation always expects CAS zero and receives generation one.
    #[must_use]
    pub fn first_operation_id(&self) -> Uuid {
        Uuid::new_v5(
            &self.intent.creation().fields().operation_id.as_uuid(),
            b"heph-owned-first-provision-v1\0",
        )
    }
    /// Binds the database-loaded original receipt to these exact checked inputs.
    ///
    /// # Errors
    /// Rejects a different registration or invalid original receipt fingerprints.
    pub fn purpose(
        &self,
        receipt: OwnedVolumeRegistrationReceipt,
    ) -> Result<OwnedBackingPurpose, VolumeError> {
        if receipt.intent != self.intent {
            return Err(VolumeError::IntentConflict);
        }
        OwnedBackingPurpose::new(
            receipt,
            self.host.clone(),
            self.root.clone(),
            self.owner_namespace,
            1,
        )
    }
}

/// Global database history classification, without filesystem or format authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedProvisioningDiscovery {
    /// Positive original 110 birth, CAS zero and no admitted 111 history.
    ///
    /// This is not physical absence, a new claim or permission to format.
    Unadmitted(Box<OwnedVolumeRegistrationReceipt>),
    /// Exact original first operation, retaining its original progress and fence.
    ///
    /// A pending or uncertain observation must not become a new operation.
    Original(Box<OwnedProvisioningContext>),
}

#[cfg(test)]
mod tests;

//! Explicit sealed birth path on the canonical local store; no recipe authority.

mod execution;
mod filesystem;
pub mod partial_retention;
#[cfg(test)]
mod tests;

use crate::{LocalVolumeStore, VolumeRootOwner};
use identity_domain::AuthenticatedIdentity;
use std::sync::Arc;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPurpose, OwnedProvisioningClaim, OwnedProvisioningContext,
    VolumeError, VolumeOwnedProvisioningRepository,
};

/// Explicit actor admission and independent worker recording over one volume repo.
///
/// Role separation is checked before bytes are created. A supplied historical
/// claim or correlation label grants no new actor effect permission.
#[derive(Clone)]
pub struct OwnedVolumeMetadata {
    actor: Arc<dyn VolumeOwnedProvisioningRepository>,
    worker: Arc<dyn VolumeOwnedProvisioningRepository>,
    owner: Arc<VolumeRootOwner>,
}
impl OwnedVolumeMetadata {
    /// Composes distinct actor and worker adapters plus the actual volume-root guard.
    #[must_use]
    pub const fn new(
        actor: Arc<dyn VolumeOwnedProvisioningRepository>,
        worker: Arc<dyn VolumeOwnedProvisioningRepository>,
        owner: Arc<VolumeRootOwner>,
    ) -> Self {
        Self {
            actor,
            worker,
            owner,
        }
    }
}
impl LocalVolumeStore {
    /// Enables the sealed path explicitly; legacy generic methods stay separate.
    ///
    /// # Errors
    /// Rejects a root or host differing from the persisted physical owner binding.
    pub fn with_owned_metadata(
        mut self,
        metadata: OwnedVolumeMetadata,
    ) -> Result<Self, VolumeError> {
        if metadata.owner.root() != self.config.volume_root
            || metadata.owner.host() != self.config.host_id
        {
            return Err(VolumeError::IntentConflict);
        }
        metadata.owner.validate()?;
        self.owned_metadata = Some(metadata);
        Ok(self)
    }
    /// Creates/formats only an exact sealed birth under fresh original-actor admission.
    ///
    /// The trusted engine MUST compare the authoritative preceding Create claim
    /// before this call; metadata correlation alone cannot prove recipe authority.
    /// Same-operation replay rechecks the live actor before any first-format work.
    ///
    /// # Errors
    /// Rejects missing composition, permissions, purpose/CAS conflicts and ambiguous IO.
    pub async fn provision_owned(
        &self,
        identity: &AuthenticatedIdentity,
        request: &BeginOwnedProvisioning,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        let metadata = self.owned_composition(&request.purpose)?;
        let admitted = metadata
            .actor
            .begin_owned_provisioning(identity, request)
            .await?;
        if admitted.claim.purpose != request.purpose
            || admitted.claim.operation_id != request.operation_id
        {
            return Err(VolumeError::IntentConflict);
        }
        let current = metadata
            .worker
            .owned_reconciliation_context(&admitted.claim)
            .await?;
        if current.claim != admitted.claim {
            return Err(VolumeError::IntentConflict);
        }
        metadata.owner.validate()?;
        execution::provision(self, metadata, identity, request, current).await
    }
    /// Readonly physical recovery, including after creator or source permission loss.
    ///
    /// Never creates files, allocates, completes journal transitions or formats.
    /// An unfinished/unknown birth remains held; neither expiry nor row absence
    /// proves host absence. Initial Ready requires exact clean physical birth proof.
    ///
    /// # Errors
    /// Rejects unavailable worker composition, missing/contradictory evidence or dirtiness.
    pub async fn reconcile_owned(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        let metadata = self.owned_composition(&claim.purpose)?;
        let current = metadata.worker.owned_reconciliation_context(claim).await?;
        if current.claim != *claim {
            return Err(VolumeError::IntentConflict);
        }
        metadata.owner.validate()?;
        execution::reconcile(self, metadata, current).await
    }
    fn owned_composition(
        &self,
        purpose: &OwnedBackingPurpose,
    ) -> Result<&OwnedVolumeMetadata, VolumeError> {
        let metadata = self
            .owned_metadata
            .as_ref()
            .ok_or(VolumeError::InvalidState(
                "sealed owned provisioning metadata is not configured",
            ))?;
        metadata.owner.validate()?;
        if purpose.host() != self.config.host_id
            || purpose.root() != self.config.volume_root
            || purpose.owner_namespace() != metadata.owner.namespace_id()
        {
            return Err(VolumeError::IntentConflict);
        }
        Ok(metadata)
    }
}

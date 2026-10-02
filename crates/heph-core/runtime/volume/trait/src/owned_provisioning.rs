//! Canonical sealed provisioning metadata; values never prove host IO on their own.

mod observation;
mod purpose;
#[cfg(test)]
mod tests;

pub use observation::{OwnedBackingObservation, OwnedBackingObservationWire, OwnedFilesystemBirth};
pub use purpose::OwnedBackingPurpose;
pub use volume_domain::{OwnedBackingPhase, VolumeRootNamespaceId};

use crate::VolumeError;
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use uuid::Uuid;

/// One stable actor operation, with immutable purpose and expected current fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginOwnedProvisioning {
    /// Exact immutable backing birth purpose.
    pub purpose: OwnedBackingPurpose,
    /// Stable operation identity; ambiguity permits only exact replay.
    pub operation_id: Uuid,
    /// Current provisioning generation observed before admission.
    pub expected_generation: u64,
}

/// Original committed actor operation, distinct from immutable metadata birth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedProvisioningClaim {
    /// Exact purpose retained across CAS retries.
    pub purpose: OwnedBackingPurpose,
    /// Stable operation admitted with live original-actor project authority.
    pub operation_id: Uuid,
    /// Current generation allocated to this operation.
    pub generation: u64,
    /// Exact original operation request provenance.
    pub request_id: Uuid,
    /// Exact committed admission event.
    pub event_id: Uuid,
}

/// Readonly progress. Historical admission is never new format authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedProvisioningContext {
    /// Exact admitted operation and fence.
    pub claim: OwnedProvisioningClaim,
    /// Latest authoritative worker observation, if any.
    pub observation: Option<OwnedBackingObservation>,
}

/// Sealed backing operations on the same canonical metadata repository.
#[async_trait]
pub trait VolumeOwnedProvisioningRepository: Send + Sync + 'static {
    /// Admits a fresh original-actor operation or returns its exact original receipt.
    ///
    /// Project management is live before replay. The adapter exactly compares the
    /// original seal/receipt/correlation. Trusted engines must separately verify
    /// their authoritative preceding Create claim; labels confer no such authority.
    /// Ambiguous operations block different operations. Ready never permits creation.
    async fn begin_owned_provisioning(
        &self,
        identity: &AuthenticatedIdentity,
        request: &BeginOwnedProvisioning,
    ) -> Result<OwnedProvisioningContext, VolumeError>;
    /// Worker-only exact readonly context; never reconstructs an OIDC identity.
    ///
    /// Survives creator role loss, but conveys no new first-format permission.
    async fn owned_reconciliation_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedProvisioningContext, VolumeError>;
    /// Worker-only exact CAS observation, committed atomically with metadata/event.
    ///
    /// Callers must verify the physical root marker, OS lock, complete journal,
    /// exact birth inode and readonly clean UUID/geometry before submitting Ready.
    /// This metadata boundary accepts no actor DTO as proof of those host actions.
    async fn record_owned_observation(
        &self,
        claim: &OwnedProvisioningClaim,
        observation: &OwnedBackingObservation,
    ) -> Result<OwnedProvisioningContext, VolumeError>;
}

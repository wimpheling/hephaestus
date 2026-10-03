//! Distinct worker comparison facts for closed, quiescent Ready retention.
//!
//! These metadata DTOs prove neither an OS lock nor VM shutdown. The owning
//! coordinator retains actual Local custody through worker and actor commits.

use crate::{OwnedBackingObservation, OwnedBackingPhase, OwnedProvisioningContext, VolumeError};
use async_trait::async_trait;
use runtime_types::VolumeId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Untrusted durable closure projection; all fields are compared again by adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedReadyRetentionClosureWire {
    /// Exact immutable closure identity.
    pub id: Uuid,
    /// Exact sealed volume.
    pub volume_id: VolumeId,
    /// Exact owning project, never an authority claim.
    pub project_id: Uuid,
    /// Immutable original sealed registration operation.
    pub creation_operation_id: Uuid,
    /// Original first provisioning operation.
    pub provisioning_operation_id: Uuid,
    /// Original provisioning fence; no new CAS is allocated for retention.
    pub generation: u64,
    /// Exact immutable original purpose hash.
    pub purpose_hash: [u8; 32],
    /// Exact original positive Ready observation.
    pub ready_observation_id: Uuid,
    /// Trusted configured actual VM namespace comparison pin, distinct from disk ownership.
    pub vm_provider_namespace: String,
    /// Trusted configured actual VM host comparison pin, never supplied by clients.
    pub vm_host_id: String,
}
/// Structurally checked backend comparison data; not physical or actor authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OwnedReadyRetentionClosure(OwnedReadyRetentionClosureWire);
impl OwnedReadyRetentionClosure {
    /// Returns validated comparison fields.
    #[must_use]
    pub const fn fields(&self) -> &OwnedReadyRetentionClosureWire {
        &self.0
    }
}
impl TryFrom<OwnedReadyRetentionClosureWire> for OwnedReadyRetentionClosure {
    type Error = VolumeError;
    fn try_from(wire: OwnedReadyRetentionClosureWire) -> Result<Self, Self::Error> {
        if [
            wire.id,
            wire.volume_id.as_uuid(),
            wire.project_id,
            wire.creation_operation_id,
            wire.provisioning_operation_id,
            wire.ready_observation_id,
        ]
        .iter()
        .any(Uuid::is_nil)
            || wire.generation != 1
            || wire.vm_provider_namespace.is_empty()
            || wire.vm_provider_namespace.len() > 128
            || matches!(wire.vm_provider_namespace.as_str(), "." | "..")
            || !wire.vm_provider_namespace.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            })
            || wire.vm_host_id.is_empty()
            || wire.vm_host_id.len() > 128
            || matches!(wire.vm_host_id.as_str(), "." | "..")
            || !wire.vm_host_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            })
        {
            return Err(VolumeError::IntentConflict);
        }
        Ok(Self(wire))
    }
}
/// Exact readonly closed context, including immutable initial Ready evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedReadyRetentionContext {
    /// Durable immutable closure; its existence never proves physical quiescence.
    pub closure: OwnedReadyRetentionClosure,
    /// Exact original operation and positive birth proof.
    pub original: OwnedProvisioningContext,
}
impl OwnedReadyRetentionContext {
    /// Compares a projected closure with the full original checked purpose.
    ///
    /// # Errors
    /// Foreign volumes, operations, original observations or purpose pins deny.
    pub fn validate(&self) -> Result<(), VolumeError> {
        let fields = self.closure.fields();
        let claim = &self.original.claim;
        let registration = claim.purpose.receipt().intent.registration();
        let observation = self
            .original
            .observation
            .as_ref()
            .ok_or(VolumeError::IntentConflict)?;
        if fields.volume_id != registration.id()
            || fields.project_id != registration.project_id()
            || fields.creation_operation_id
                != claim
                    .purpose
                    .receipt()
                    .intent
                    .creation()
                    .fields()
                    .operation_id
                    .as_uuid()
            || fields.provisioning_operation_id != claim.operation_id
            || fields.generation != claim.generation
            || fields.purpose_hash != claim.purpose.hash()
            || fields.ready_observation_id != observation.fields().id
            || observation.fields().phase != OwnedBackingPhase::Ready
        {
            return Err(VolumeError::IntentConflict);
        }
        Ok(())
    }
}
/// Positive readonly retention observation; never appended as a new original birth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedReadyRetentionFact {
    /// Exact immutable new worker fact.
    pub id: Uuid,
    /// Exact immutable closure.
    pub closure_id: Uuid,
    /// Actual readonly observation of current clean backing identity and geometry.
    pub observation: OwnedBackingObservation,
    /// Committed worker event; grants no manager or Source rights.
    pub event_id: Uuid,
}
/// Worker-only closed Ready comparison and observation boundary.
#[async_trait]
pub trait VolumeReadyRetentionRepository: Send + Sync + 'static {
    /// Reads global consumer/lease history and exact closed original context.
    ///
    /// # Errors
    /// Unsupported, open/reopened, foreign or unproven history remains held.
    async fn ready_retention_context(
        &self,
        _claim: &crate::OwnedProvisioningClaim,
        _closure_id: Uuid,
    ) -> Result<OwnedReadyRetentionContext, VolumeError> {
        Err(VolumeError::InvalidState("Ready retention is unsupported"))
    }
    /// Records an exact new readonly fact while actual Local custody is retained.
    ///
    /// It does not alter the original Ready evidence or provisioning generation.
    ///
    /// # Errors
    /// Unsupported, changed context, contradictory physical identity or lease history denies.
    async fn record_ready_retention_observation(
        &self,
        _context: &OwnedReadyRetentionContext,
        _observation: &OwnedBackingObservation,
    ) -> Result<OwnedReadyRetentionFact, VolumeError> {
        Err(VolumeError::InvalidState(
            "Ready retention recording is unsupported",
        ))
    }
}

#[cfg(test)]
#[path = "ready_retention/tests.rs"]
mod tests;

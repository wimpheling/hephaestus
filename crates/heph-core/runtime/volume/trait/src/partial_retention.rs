//! Worker partial-evidence metadata boundary; actor SQL integration remains separate.
//!
//! No port here commits an actor fence independently of its recipe transaction.
//! The Local provider supplies concrete opaque partial-birth custody. The owning
//! service MUST retain that actual guard across worker receipt recording and the
//! actor's atomic fence/ledger commit.
//! These DTOs, timestamps, database roles and receipt labels cannot establish
//! custody of an OS flock. The worker `PostgreSQL` adapter, atomic actor service and
//! public migration remain pending. Publication requires actual native custody and
//! atomic fence/ledger integration tests; DTO tests cannot satisfy those gates.

use async_trait::async_trait;
use uuid::Uuid;
use volume_domain::{OwnedPartialBirthObservation, OwnedPartialRetentionReceiptId};

use crate::{OwnedBackingPhase, OwnedProvisioningClaim, VolumeError};

/// Exact authoritative current observation head, independent of provisioning CAS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialRetentionObservationHead {
    /// Monotonic version; zero means there has been no worker observation.
    pub version: u64,
    /// Exact current immutable worker row, absent only at version zero.
    pub observation_id: Option<Uuid>,
    /// Latest 111 phase. A prior partial receipt is not a new format permission.
    pub backing_phase: Option<OwnedBackingPhase>,
    /// Prior positive partial receipt if it is the current head instead.
    pub partial_receipt_id: Option<OwnedPartialRetentionReceiptId>,
}

/// Worker-only comparison context; history never grants a new actor effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPartialRetentionContext {
    /// Exact original sealed provisioning admission, not a reconstructed identity.
    pub claim: OwnedProvisioningClaim,
    /// Exact current physical observation CAS; the adapter rechecks on recording.
    pub head: PartialRetentionObservationHead,
}

/// Immutable backend receipt. Public client Remove inputs must never accept it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPartialRetentionReceipt {
    /// Worker row identifier; lookup labels alone cannot authorize retirement.
    pub id: OwnedPartialRetentionReceiptId,
    /// Exact original admission matched when this receipt was recorded.
    pub claim: OwnedProvisioningClaim,
    /// Checked positive facts, never Ready, host absence or deletion proof.
    pub observation: OwnedPartialBirthObservation,
    /// Exact committed worker project event, for audit rather than actor grants.
    pub event_id: Uuid,
}

/// Separate worker-only extension on the canonical volume metadata repository.
///
/// Every method rejects permanently retired births. The implementation must
/// validate exact full purpose, seal, original effect identity and current head.
/// No actor can mint a receipt through this port, and no returned value proves
/// physical custody. The future service obtains and retains the actual Local
/// held witness; no placeholder `lock_held` trait, flag or heartbeat is accepted.
#[async_trait]
pub trait VolumePartialRetentionRepository: Send + Sync + 'static {
    /// Reads original admitted identity/current head without first-format authority.
    async fn partial_retention_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedPartialRetentionContext, VolumeError>;

    /// Appends exact positive partial evidence and advances the observation head CAS.
    ///
    /// The physical witness remains held through the later actor transaction.
    /// Recording never changes filesystem readiness or provisioning generation.
    async fn record_partial_retention_observation(
        &self,
        context: &OwnedPartialRetentionContext,
        observation: &OwnedPartialBirthObservation,
    ) -> Result<OwnedPartialRetentionReceipt, VolumeError>;
}

//! Readonly physical observation; actor fencing and recipe orchestration are separate.

use super::filesystem;
use crate::{
    LocalVolumeStore,
    owned_journal::partial_retention::{OwnedPartialBirthCustody, unavailable},
};
use std::{fmt, sync::Arc, time::Duration};
use volume_trait::{
    OwnedBackingPhase, OwnedPartialBirthPhase, OwnedProvisioningClaim,
    PartialRetentionObservationHead, VolumeError, VolumePartialRetentionRepository,
};

/// Bounded observation failure; no branch creates files or guesses host absence.
#[derive(Debug)]
pub enum PartialBirthCustodyError {
    /// A formatter/other observer owns the actual nonblocking per-volume flock.
    Unavailable,
    /// Metadata, positive birth or readonly filesystem evidence is unavailable/conflicting.
    Held(VolumeError),
}
impl fmt::Display for PartialBirthCustodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("partial birth custody is unavailable"),
            Self::Held(error) => write!(f, "partial birth remains held: {error}"),
        }
    }
}
impl std::error::Error for PartialBirthCustodyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable => None,
            Self::Held(error) => Some(error),
        }
    }
}
impl From<VolumeError> for PartialBirthCustodyError {
    fn from(error: VolumeError) -> Self {
        if unavailable(&error) {
            Self::Unavailable
        } else {
            Self::Held(error)
        }
    }
}
impl LocalVolumeStore {
    /// Observes a positive owned partial birth while retaining its actual flock custody.
    ///
    /// Only server composition supplies the worker adapter. Historical actor/source
    /// rights are not reconstructed. The returned nonclone guard must remain held
    /// through future worker recording and atomic actor fence/ledger commit; this
    /// method neither records a receipt nor performs that transaction.
    ///
    /// # Errors
    /// Returns `Unavailable` immediately for an active formatter/holder; otherwise
    /// holds missing/foreign/Ready/contradictory births without filesystem writes.
    pub async fn observe_owned_partial(
        &self,
        worker: &dyn VolumePartialRetentionRepository,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedPartialBirthCustody, PartialBirthCustodyError> {
        let metadata = self.owned_composition(&claim.purpose)?;
        let prior = metadata.worker.owned_reconciliation_context(claim).await?;
        let context = worker.partial_retention_context(claim).await?;
        if prior.claim != *claim
            || context.claim != *claim
            || !valid_head(&context.head)
            || prior
                .observation
                .as_ref()
                .is_some_and(|proof| proof.fields().phase == OwnedBackingPhase::Ready)
        {
            return Err(VolumeError::IntentConflict.into());
        }
        let mut custody = OwnedPartialBirthCustody::acquire(
            Arc::clone(&metadata.owner),
            &context,
            prior.observation.as_ref(),
        )?;
        if custody.observation().fields().phase == OwnedPartialBirthPhase::FormatIntentIncomplete {
            let registration = claim.purpose.receipt().intent.registration();
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                filesystem::prove(
                    custody.file(),
                    custody.child_lock()?,
                    registration.capacity_bytes(),
                    registration.filesystem_uuid(),
                    &self.config.mkfs_ext4,
                ),
            )
            .await
            .map_err(|_| VolumeError::ProvisioningUncertain("readonly partial probe timed out"))?;
            match result {
                Ok(_) => {
                    return Err(VolumeError::InvalidState(
                        "clean Ready birth uses normal reconciliation",
                    )
                    .into());
                }
                Err(VolumeError::ProvisioningUncertain(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
        custody.revalidate()?;
        Ok(custody)
    }
}
fn valid_head(head: &PartialRetentionObservationHead) -> bool {
    if head.version == 0 {
        head.observation_id.is_none()
            && head.backing_phase.is_none()
            && head.partial_receipt_id.is_none()
    } else {
        head.version < i64::MAX.cast_unsigned()
            && head.observation_id.is_some_and(|id| !id.is_nil())
            && match (head.backing_phase, head.partial_receipt_id) {
                (Some(phase), None) => phase != OwnedBackingPhase::Ready,
                (None, Some(receipt)) => Some(receipt.as_uuid()) == head.observation_id,
                _ => false,
            }
    }
}

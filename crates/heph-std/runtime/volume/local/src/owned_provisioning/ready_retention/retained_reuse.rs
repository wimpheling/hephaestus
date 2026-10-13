//! Physical reuse custody remains separate from the original Remove context.

use super::{OwnedReadyRetentionCustody, changes, observe, recover};
use crate::{LocalVolumeStore, backing, owned_journal::JournalPurpose};
use std::sync::Arc;
use volume_trait::{
    OwnedBackingObservation, OwnedRetainedReuseContext, VolumeError, VolumeRetainedReuseRepository,
};

/// Actual nonclone original journal custody for a new retained external binding.
///
/// Metadata and readonly observation grant no actor or mount authority. Move this
/// guard into protected completion before awaiting it; retain it until the worker
/// and actor outcomes are resolved. The private inner guard cannot be extracted.
///
/// ```compile_fail
/// fn cannot_clone(guard: &volume_local::OwnedRetainedReuseCustody) {
///     let _: volume_local::OwnedRetainedReuseCustody = guard.clone();
/// }
/// ```
#[must_use = "retain actual journal custody through protected retained reuse completion"]
pub struct OwnedRetainedReuseCustody {
    inner: OwnedReadyRetentionCustody,
    context: OwnedRetainedReuseContext,
}
impl OwnedRetainedReuseCustody {
    /// Exact compared retained context, without current actor or physical authority.
    #[must_use]
    pub const fn context(&self) -> &OwnedRetainedReuseContext {
        &self.context
    }

    /// Current complete readonly physical observation under actual journal custody.
    #[must_use]
    pub const fn observation(&self) -> &OwnedBackingObservation {
        self.inner.observation()
    }

    /// Rechecks actual root, journal, backing inode and unchanged file metadata.
    ///
    /// This does not recheck database admission or repeat the filesystem child.
    ///
    /// # Errors
    /// Replaced objects, changed original context or file mutation deny completion.
    pub fn revalidate(&mut self) -> Result<(), VolumeError> {
        self.context.validate()?;
        if self.inner.context() != &self.context.retained {
            return Err(VolumeError::IntentConflict);
        }
        self.inner.revalidate()
    }
}
impl LocalVolumeStore {
    /// Observes genuinely retained completed backing under the existing journal flock.
    ///
    /// Requalifies the full distinct worker context before and after the bounded
    /// readonly child. The child inherits the actual flock cancellation contract.
    /// No root initialization, adoption, creation, format, original history writes,
    /// provisioning CAS, worker fact or current actor grants occur.
    ///
    /// # Errors
    /// Missing composition, foreign purpose/owner, absent completed retention,
    /// reopened or unquiesced history, changed backing or dirty/timed-out probe denies.
    pub async fn observe_retained_owned_reuse(
        &self,
        worker: &dyn VolumeRetainedReuseRepository,
        expected: &OwnedRetainedReuseContext,
    ) -> Result<OwnedRetainedReuseCustody, VolumeError> {
        expected.validate()?;
        let claim = &expected.retained.original.claim;
        let metadata = self.owned_composition(&claim.purpose)?;
        let purpose = &claim.purpose;
        let journal_purpose = JournalPurpose::new(
            purpose.receipt(),
            purpose.host(),
            purpose.root(),
            purpose.owner_namespace(),
            purpose.birth_generation(),
        )
        .map_err(backing)?;
        let mut lock = metadata.owner.journal_lock(journal_purpose, true)?;
        let current = worker
            .retained_reuse_context(
                claim,
                expected.retained.closure.fields().id,
                expected.retention_operation_id,
            )
            .await?;
        if current != *expected {
            return Err(VolumeError::IntentConflict);
        }
        let file = recover(&mut lock, &current.retained)?;
        let unchanged = changes(file.file())?;
        let observation = observe(self, &mut lock, &current.retained, &file).await?;
        let prior = current
            .retained
            .original
            .observation
            .as_ref()
            .ok_or(VolumeError::IntentConflict)?;
        let mut compared = observation.fields().clone();
        compared.id = prior.fields().id;
        if compared != *prior.fields() || changes(file.file())? != unchanged {
            return Err(VolumeError::ProvisioningUncertain(
                "retained reuse backing differs from original or changed during probe",
            ));
        }
        let after = worker
            .retained_reuse_context(
                claim,
                current.retained.closure.fields().id,
                current.retention_operation_id,
            )
            .await?;
        if after != current {
            return Err(VolumeError::IntentConflict);
        }
        let mut custody = OwnedRetainedReuseCustody {
            inner: OwnedReadyRetentionCustody {
                owner: Arc::clone(&metadata.owner),
                lock,
                changes: unchanged,
                backing: file,
                context: current.retained.clone(),
                observation,
            },
            context: current,
        };
        custody.revalidate()?;
        Ok(custody)
    }
}

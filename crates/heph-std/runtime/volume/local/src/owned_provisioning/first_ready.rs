//! First-consumer verification with actual non-clone journal custody.

use super::filesystem;
use crate::{
    LocalVolumeStore, VolumeRootOwner, backing,
    owned_journal::{JournalPurpose, OwnedJournal, OwnedJournalLock, RecordedBacking},
};
use std::{os::unix::fs::MetadataExt, sync::Arc, time::Duration};
use volume_trait::{
    OwnedBackingPhase, OwnedProvisioningClaim, OwnedProvisioningContext, VolumeError,
};

/// Actual first-birth journal flock retained through protected106 completion.
///
/// This is neither a runtime lease nor a reusable filesystem receipt. Only the
/// local provider can construct it. Keep it alive through worker recording and
/// actor completion, including cancellation while a transaction may still commit.
/// Completed106 replay must bypass this verifier; it cannot fsck a runtime volume.
///
/// ```compile_fail
/// fn cannot_clone(guard: &volume_local::OwnedReadyCustody) {
///     let _: volume_local::OwnedReadyCustody = guard.clone();
/// }
/// ```
#[must_use = "retain actual journal custody through first completion"]
pub struct OwnedReadyCustody {
    owner: Arc<VolumeRootOwner>,
    lock: OwnedJournalLock,
    backing: RecordedBacking,
    context: OwnedProvisioningContext,
    changes: [i64; 4],
}
impl OwnedReadyCustody {
    /// Original worker context; this DTO alone is not physical verification.
    #[must_use]
    pub const fn context(&self) -> &OwnedProvisioningContext {
        &self.context
    }

    /// Rechecks pinned root, journal, inode and unchanged file metadata under custody.
    ///
    /// Call before recording and actor completion. Once completion has committed,
    /// runtime admission may open, so a later check must not run another fsck.
    ///
    /// # Errors
    /// Replaced roots, paths, journal bytes, inodes or file changes remain held.
    pub fn revalidate(&mut self) -> Result<(), VolumeError> {
        self.owner.validate()?;
        let journal = OwnedJournal::open_existing(&mut self.lock).map_err(backing)?;
        let prior = self
            .context
            .observation
            .as_ref()
            .ok_or(VolumeError::IntentConflict)?;
        journal.validate_history(prior).map_err(backing)?;
        let current = journal.recover_readonly().map_err(backing)?;
        let mut observed = journal
            .observe(
                &current,
                &self.context.claim,
                prior.fields().filesystem.clone(),
            )
            .map_err(backing)?
            .fields()
            .clone();
        observed.id = prior.fields().id;
        if current.identity() != self.backing.identity()
            || observed != *prior.fields()
            || changes(current.file())? != self.changes
            || changes(self.backing.file())? != self.changes
        {
            return Err(VolumeError::ProvisioningUncertain(
                "first Ready birth changed during custody",
            ));
        }
        self.owner.validate()
    }
}
impl LocalVolumeStore {
    /// Verifies a Ready birth before its original protected Create completes.
    ///
    /// Requires the dedicated durable admission fence and GLOBAL zero runtime
    /// lease history under the actual journal flock. The fence blocks every lease
    /// acquisition until exact immutable106 Applied/Ready completion. The child
    /// inherits this lock, so cancellation retains exclusion until it exits.
    /// No metadata Ready rewrite, file creation, format, adoption or root bootstrap
    /// occurs. The returned guard must survive the entire completion transaction.
    ///
    /// # Errors
    /// Denies unsupported/completed/leased/foreign/retired births, conflicting
    /// journal or inode evidence, dirty geometry, active custody and fsck timeout.
    pub async fn verify_owned_ready(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedReadyCustody, VolumeError> {
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
        // A stale pre-lock Ready result cannot authorize fsck after another
        // creator completes106 and opens runtime admission.
        metadata
            .worker
            .assert_owned_first_ready_verification(claim)
            .await?;
        let current = metadata.worker.owned_reconciliation_context(claim).await?;
        let prior = current
            .observation
            .as_ref()
            .ok_or(VolumeError::IntentConflict)?;
        if current.claim != *claim || prior.fields().phase != OwnedBackingPhase::Ready {
            return Err(VolumeError::IntentConflict);
        }
        let file = {
            let journal = OwnedJournal::open_existing(&mut lock).map_err(backing)?;
            journal.validate_history(prior).map_err(backing)?;
            if !journal.allocation_complete().map_err(backing)?
                || !journal.publication_complete().map_err(backing)?
                || !journal.format_intent_present().map_err(backing)?
            {
                return Err(VolumeError::IntentConflict);
            }
            journal.recover_readonly().map_err(backing)?
        };
        let registration = purpose.receipt().intent.registration();
        let birth = tokio::time::timeout(
            Duration::from_secs(30),
            filesystem::prove(
                file.file(),
                lock.child_lock().map_err(backing)?,
                registration.capacity_bytes(),
                registration.filesystem_uuid(),
                &self.config.mkfs_ext4,
            ),
        )
        .await
        .map_err(|_| {
            VolumeError::ProvisioningUncertain("first Ready readonly probe timed out")
        })??;
        if prior.fields().filesystem.as_ref() != Some(&birth) {
            return Err(VolumeError::IntentConflict);
        }
        metadata
            .worker
            .assert_owned_first_ready_verification(claim)
            .await?;
        let mut custody = OwnedReadyCustody {
            owner: Arc::clone(&metadata.owner),
            lock,
            changes: changes(file.file())?,
            backing: file,
            context: current,
        };
        custody.revalidate()?;
        Ok(custody)
    }
}
fn changes(file: &std::fs::File) -> Result<[i64; 4], VolumeError> {
    let metadata = file.metadata().map_err(backing)?;
    Ok([
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    ])
}

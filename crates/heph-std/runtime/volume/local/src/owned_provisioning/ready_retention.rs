//! Readonly Ready retention; closed admission and actual journal custody are separate.

mod retained_reuse;
pub use retained_reuse::OwnedRetainedReuseCustody;

use super::filesystem;
use crate::{
    LocalVolumeStore, VolumeRootOwner, backing,
    owned_journal::{JournalPurpose, OwnedJournal, OwnedJournalLock, RecordedBacking},
};
use std::{os::unix::fs::MetadataExt, sync::Arc, time::Duration};
use volume_trait::{
    OwnedBackingObservation, OwnedReadyRetentionContext, VolumeError,
    VolumeReadyRetentionRepository,
};

/// Actual non-clone native custody retained until fact and Remove commit resolve.
///
/// This cannot be serialized into a receipt or supplied by a client. Runtime
/// history may exist; only exact released, scoped terminal histories qualify.
///
/// ```compile_fail
/// fn cannot_clone(guard: &volume_local::OwnedReadyRetentionCustody) {
///     let _: volume_local::OwnedReadyRetentionCustody = guard.clone();
/// }
/// ```
#[must_use = "retain actual journal custody through the Ready retention commit"]
pub struct OwnedReadyRetentionCustody {
    owner: Arc<VolumeRootOwner>,
    lock: OwnedJournalLock,
    backing: RecordedBacking,
    context: OwnedReadyRetentionContext,
    observation: OwnedBackingObservation,
    changes: [i64; 4],
}
impl OwnedReadyRetentionCustody {
    /// Original backend context; does not itself prove native custody.
    #[must_use]
    pub const fn context(&self) -> &OwnedReadyRetentionContext {
        &self.context
    }
    /// Current readonly clean physical evidence under actual custody.
    #[must_use]
    pub const fn observation(&self) -> &OwnedBackingObservation {
        &self.observation
    }
    /// Rechecks actual root/journal/inode and unchanged current file metadata.
    ///
    /// # Errors
    /// Any replaced object, journal difference or mutation remains held.
    pub fn revalidate(&mut self) -> Result<(), VolumeError> {
        self.owner.validate()?;
        self.context.validate()?;
        let journal = OwnedJournal::open_existing(&mut self.lock).map_err(backing)?;
        let prior = self
            .context
            .original
            .observation
            .as_ref()
            .ok_or(VolumeError::IntentConflict)?;
        journal.validate_history(prior).map_err(backing)?;
        let current = journal.recover_readonly().map_err(backing)?;
        let mut fields = journal
            .observe(
                &current,
                &self.context.original.claim,
                self.observation.fields().filesystem.clone(),
            )
            .map_err(backing)?
            .fields()
            .clone();
        fields.id = self.observation.fields().id;
        if current.identity() != self.backing.identity()
            || fields != *self.observation.fields()
            || changes(current.file())? != self.changes
            || changes(self.backing.file())? != self.changes
        {
            return Err(VolumeError::ProvisioningUncertain(
                "Ready retention backing changed during custody",
            ));
        }
        self.owner.validate()
    }
}
impl LocalVolumeStore {
    /// Observes a closed, globally quiescent original Ready birth without writes.
    ///
    /// Rechecks the worker context UNDER the existing journal lock. The durable
    /// guard excludes acquisition/renewal and all future original provisioning IO.
    /// Global old lease history must be genuinely Released with qualified scoped
    /// terminal proof. Unknown imports or foreign/null-Run history remain held.
    /// No root initialization, creation, format, metadata Ready rewrite or new CAS
    /// occurs. The bounded readonly child inherits the actual native lock.
    ///
    /// # Errors
    /// Active custody, unsupported metadata, changed context/root/inode/UUID,
    /// unquiesced history, dirtiness or readonly check timeout denies retention.
    pub async fn observe_owned_ready_retention(
        &self,
        worker: &dyn VolumeReadyRetentionRepository,
        expected: &OwnedReadyRetentionContext,
    ) -> Result<OwnedReadyRetentionCustody, VolumeError> {
        expected.validate()?;
        let claim = &expected.original.claim;
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
            .ready_retention_context(claim, expected.closure.fields().id)
            .await?;
        if current != *expected {
            return Err(VolumeError::IntentConflict);
        }
        let file = recover(&mut lock, &current)?;
        let observation = observe(self, &mut lock, &current, &file).await?;
        let after = worker
            .ready_retention_context(claim, current.closure.fields().id)
            .await?;
        if after != current {
            return Err(VolumeError::IntentConflict);
        }
        let mut custody = OwnedReadyRetentionCustody {
            owner: Arc::clone(&metadata.owner),
            lock,
            changes: changes(file.file())?,
            backing: file,
            context: current,
            observation,
        };
        custody.revalidate()?;
        Ok(custody)
    }
}
fn recover(
    lock: &mut OwnedJournalLock,
    current: &OwnedReadyRetentionContext,
) -> Result<RecordedBacking, VolumeError> {
    let prior = current
        .original
        .observation
        .as_ref()
        .ok_or(VolumeError::IntentConflict)?;
    let journal = OwnedJournal::open_existing(lock).map_err(backing)?;
    journal.validate_history(prior).map_err(backing)?;
    if !journal.allocation_complete().map_err(backing)?
        || !journal.publication_complete().map_err(backing)?
        || !journal.format_intent_present().map_err(backing)?
    {
        return Err(VolumeError::IntentConflict);
    }
    journal.recover_readonly().map_err(backing)
}

async fn observe(
    store: &LocalVolumeStore,
    lock: &mut OwnedJournalLock,
    current: &OwnedReadyRetentionContext,
    file: &RecordedBacking,
) -> Result<OwnedBackingObservation, VolumeError> {
    let prior = current
        .original
        .observation
        .as_ref()
        .ok_or(VolumeError::IntentConflict)?;
    let claim = &current.original.claim;
    let registration = claim.purpose.receipt().intent.registration();
    let filesystem = tokio::time::timeout(
        Duration::from_secs(30),
        filesystem::prove(
            file.file(),
            lock.child_lock().map_err(backing)?,
            registration.capacity_bytes(),
            registration.filesystem_uuid(),
            &store.config.mkfs_ext4,
        ),
    )
    .await
    .map_err(|_| {
        VolumeError::ProvisioningUncertain("Ready retention readonly probe timed out")
    })??;
    if prior.fields().filesystem.as_ref() != Some(&filesystem) {
        return Err(VolumeError::IntentConflict);
    }
    let observation = {
        let journal = OwnedJournal::open_existing(lock).map_err(backing)?;
        journal
            .observe(file, claim, Some(filesystem))
            .map_err(backing)?
    };
    Ok(observation)
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

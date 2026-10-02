//! Concrete readonly custody; no public constructor, cloning, or lock assertion.

mod evidence;

use super::{
    JournalError, JournalPurpose, OwnedJournal, OwnedJournalLock, RecordedBacking, filesystem,
};
use crate::{VolumeRootOwner, backing};
use std::{fs::File, os::unix::fs::MetadataExt, sync::Arc};
use volume_trait::{
    OwnedBackingObservation, OwnedPartialBirthObservation, OwnedPartialRetentionContext,
    VolumeError,
};

/// Actual native custody of one positive partial birth and its per-volume flock.
///
/// Keep this value alive through worker recording and the actor's atomic fence
/// and ledger commit. Observation DTOs can be cloned; this guard cannot. Its
/// constructor is private to the provider and never opens anything for creation.
/// Dropping it releases custody; a receipt alone cannot replace it.
///
/// ```compile_fail
/// fn cannot_duplicate(guard: &volume_local::OwnedPartialBirthCustody) {
///     let _: volume_local::OwnedPartialBirthCustody = guard.clone();
/// }
/// ```
#[must_use = "retain physical custody through worker recording and actor transaction commit"]
pub struct OwnedPartialBirthCustody {
    owner: Arc<VolumeRootOwner>,
    lock: OwnedJournalLock,
    namespace: File,
    backing: RecordedBacking,
    context: OwnedPartialRetentionContext,
    observation: OwnedPartialBirthObservation,
    changes: [i64; 4],
}
impl OwnedPartialBirthCustody {
    pub(crate) fn acquire(
        owner: Arc<VolumeRootOwner>,
        context: &OwnedPartialRetentionContext,
        previous: Option<&OwnedBackingObservation>,
    ) -> Result<Self, VolumeError> {
        owner.validate()?;
        let purpose = &context.claim.purpose;
        // Both getters refer to the checked physical volume-root namespace;
        // neither is the orchestration creation scope or a VM provider owner.
        let namespace_matches = purpose.owner_namespace() == owner.namespace_id();
        if purpose.root() != owner.root() || purpose.host() != owner.host() || !namespace_matches {
            return Err(VolumeError::IntentConflict);
        }
        let journal_purpose = JournalPurpose::new(
            purpose.receipt(),
            purpose.host(),
            purpose.root(),
            purpose.owner_namespace(),
            purpose.birth_generation(),
        )
        .map_err(backing)?;
        let mut lock = owner.journal_lock(journal_purpose, true)?;
        let (namespace, backing_file, observation) = {
            let journal = OwnedJournal::open_existing(&mut lock).map_err(backing)?;
            if let Some(previous) = previous {
                journal.validate_history(previous).map_err(backing)?;
            }
            let backing_file = journal.recover_readonly().map_err(backing)?;
            let observation =
                evidence::observe(&journal, &backing_file, context, uuid::Uuid::new_v4())
                    .map_err(backing)?;
            (
                journal.directory.try_clone().map_err(backing)?,
                backing_file,
                observation,
            )
        };
        let changes = changes(backing_file.file())?;
        owner.validate()?;
        Ok(Self {
            owner,
            lock,
            namespace,
            backing: backing_file,
            context: context.clone(),
            observation,
            changes,
        })
    }
    /// Exact worker comparison context; it confers neither actor nor IO authority.
    #[must_use]
    pub const fn context(&self) -> &OwnedPartialRetentionContext {
        &self.context
    }
    /// Checked positive facts built from the held native journal descriptors.
    #[must_use]
    pub const fn observation(&self) -> &OwnedPartialBirthObservation {
        &self.observation
    }
    /// Revalidates pinned paths, inode identity, journal bytes, phase, length and changes.
    ///
    /// # Errors
    /// Rejects root/namespace/backing replacement, writes or conflicting journal progress.
    pub fn revalidate(&mut self) -> Result<(), VolumeError> {
        self.owner.validate()?;
        filesystem::validate_directory(&self.namespace, true).map_err(backing)?;
        filesystem::validate_private_file(self.backing.file()).map_err(backing)?;
        let journal = OwnedJournal::open_existing(&mut self.lock).map_err(backing)?;
        let current = journal.recover_readonly().map_err(backing)?;
        if current.identity() != self.backing.identity()
            || filesystem::FileIdentity::of(&journal.directory).map_err(backing)?
                != filesystem::FileIdentity::of(&self.namespace).map_err(backing)?
            || changes(current.file())? != self.changes
            || changes(self.backing.file())? != self.changes
            || evidence::observe(
                &journal,
                &current,
                &self.context,
                self.observation.fields().id,
            )
            .map_err(backing)?
                != self.observation
        {
            return Err(VolumeError::ProvisioningUncertain(
                "partial birth changed during custody",
            ));
        }
        self.owner.validate()
    }
    pub(crate) const fn file(&self) -> &File {
        self.backing.file()
    }
    pub(crate) fn child_lock(&self) -> Result<File, VolumeError> {
        self.lock.child_lock().map_err(backing)
    }
}
fn changes(file: &File) -> Result<[i64; 4], VolumeError> {
    let metadata = file.metadata().map_err(backing)?;
    Ok([
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    ])
}

pub fn unavailable(error: &VolumeError) -> bool {
    matches!(error, VolumeError::Backing(source) if matches!(source.downcast_ref::<JournalError>(), Some(JournalError::Locked)))
}

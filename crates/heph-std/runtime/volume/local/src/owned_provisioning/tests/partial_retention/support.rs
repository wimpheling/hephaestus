use super::super::support::Fixture;
use crate::owned_journal::OwnedJournal;
use async_trait::async_trait;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
use volume_trait::{
    OwnedPartialBirthObservation, OwnedPartialRetentionContext, OwnedPartialRetentionReceipt,
    OwnedProvisioningClaim, PartialRetentionObservationHead, VolumeError,
    VolumePartialRetentionRepository,
};

pub struct Worker {
    pub context: OwnedPartialRetentionContext,
    pub records: AtomicUsize,
}
impl Worker {
    pub fn new(fixture: &Fixture) -> Self {
        Self {
            context: OwnedPartialRetentionContext {
                claim: fixture.claim(),
                head: PartialRetentionObservationHead {
                    version: 0,
                    observation_id: None,
                    backing_phase: None,
                    partial_receipt_id: None,
                },
            },
            records: AtomicUsize::new(0),
        }
    }
}
#[async_trait]
impl VolumePartialRetentionRepository for Worker {
    async fn partial_retention_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedPartialRetentionContext, VolumeError> {
        if claim != &self.context.claim {
            return Err(VolumeError::IntentConflict);
        }
        Ok(self.context.clone())
    }
    async fn record_partial_retention_observation(
        &self,
        _: &OwnedPartialRetentionContext,
        _: &OwnedPartialBirthObservation,
    ) -> Result<OwnedPartialRetentionReceipt, VolumeError> {
        self.records.fetch_add(1, Ordering::SeqCst);
        Err(VolumeError::InvalidState(
            "readonly observation must not record fixture receipts",
        ))
    }
}
#[derive(Clone, Copy)]
pub enum Stage {
    Empty,
    AllocationGap,
    Allocated,
    FormatIntent,
}
pub fn prepare(fixture: &Fixture, stage: Stage) {
    let mut lock = fixture
        .owner
        .journal_lock(fixture.journal_purpose(), false)
        .unwrap();
    let mut journal = OwnedJournal::open(&mut lock).unwrap();
    let backing = journal.create_staging().unwrap();
    if matches!(stage, Stage::Empty) {
        return;
    }
    journal.begin_allocation(&backing).unwrap();
    backing.file().set_len(16 * 1024 * 1024).unwrap();
    backing.file().sync_all().unwrap();
    if matches!(stage, Stage::AllocationGap) {
        return;
    }
    journal.mark_allocated(&backing).unwrap();
    journal.publish(&backing).unwrap();
    if matches!(stage, Stage::FormatIntent) {
        journal.begin_first_format(&backing).unwrap();
    }
}
pub fn snapshot(path: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            for child in fs::read_dir(entry.path()).unwrap() {
                let child = child.unwrap();
                files.insert(child.path(), fs::read(child.path()).unwrap());
            }
        } else {
            files.insert(entry.path(), fs::read(entry.path()).unwrap());
        }
    }
    files
}

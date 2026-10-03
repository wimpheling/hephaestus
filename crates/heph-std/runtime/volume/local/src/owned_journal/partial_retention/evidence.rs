use super::super::{JournalError, OwnedJournal, RecordedBacking, codec, filesystem};
use std::os::unix::fs::{FileExt, MetadataExt};
use uuid::Uuid;
use volume_trait::{
    OwnedPartialBirthObservation, OwnedPartialBirthObservationWire, OwnedPartialBirthPhase,
    OwnedPartialRetentionContext,
};

pub fn observe(
    journal: &OwnedJournal<'_>,
    backing: &RecordedBacking,
    context: &OwnedPartialRetentionContext,
    id: Uuid,
) -> Result<OwnedPartialBirthObservation, JournalError> {
    journal.validate_namespace()?;
    let phases = journal.validate_phases()?;
    let purpose = &context.claim.purpose;
    if purpose.canonical_bytes() != journal.guard.purpose().bytes() {
        return Err(JournalError::Conflict(
            "partial purpose differs from host birth",
        ));
    }
    let registration = purpose.receipt().intent.registration();
    let length = backing.file().metadata()?.len();
    let capacity = registration.capacity_bytes();
    let allocation_intent = filesystem::read_record(&journal.directory, "allocation-intent")?;
    let phase = if phases.format_intent && length == capacity {
        OwnedPartialBirthPhase::FormatIntentIncomplete
    } else if !phases.format_intent && length == 0 && !phases.allocated && !phases.published {
        OwnedPartialBirthPhase::ClaimedEmpty
    } else if !phases.format_intent
        && length == capacity
        && (phases.allocated || allocation_intent.is_some())
    {
        let mut magic = [0; 2];
        backing.file().read_exact_at(&mut magic, 1080)?;
        if magic == [0x53, 0xef] {
            return Err(JournalError::RecoveryRequired(
                "filesystem contradicts never-format birth",
            ));
        }
        OwnedPartialBirthPhase::AllocatedNeverFormat
    } else {
        return Err(JournalError::RecoveryRequired(
            "partial length/phase lacks positive birth evidence",
        ));
    };
    let mut bytes = journal.birth.hash.to_vec();
    bytes.extend_from_slice(&backing.record.hash);
    for name in [
        "allocation-intent",
        "allocated",
        "published",
        "format-intent",
    ] {
        if let Some(record) = filesystem::read_record(&journal.directory, name)? {
            bytes.push(1);
            bytes.extend_from_slice(&record);
        } else {
            bytes.push(0);
        }
    }
    OwnedPartialBirthObservation::try_from(OwnedPartialBirthObservationWire {
        id,
        observation_version: context
            .head
            .version
            .checked_add(1)
            .ok_or(JournalError::RecoveryRequired("observation CAS overflow"))?,
        volume_id: registration.id().as_uuid(),
        project_id: registration.project_id(),
        seal_id: purpose.receipt().seal_id,
        original_creation: purpose.receipt().intent.creation().fields().clone(),
        provisioning_operation_id: context.claim.operation_id,
        purpose_hash: purpose.hash(),
        generation: context.claim.generation,
        phase,
        declared_capacity_bytes: capacity,
        actual_length_bytes: length,
        owner_namespace: purpose.owner_namespace(),
        root_device: journal.birth.root.device,
        root_inode: journal.birth.root.inode,
        root_owner_uid: journal.guard.root().metadata()?.uid(),
        namespace_device: journal.birth.namespace.device,
        namespace_inode: journal.birth.namespace.inode,
        owner_uid: backing.file().metadata()?.uid(),
        backing_device: backing.record.inode.device,
        backing_inode: backing.record.inode.inode,
        birth_record_hash: journal.birth.hash,
        inode_record_hash: backing.record.hash,
        journal_hash: codec::digest(&bytes),
    })
    .map_err(|_| JournalError::RecoveryRequired("partial birth fields exceed checked bounds"))
}

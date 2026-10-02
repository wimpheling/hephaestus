use super::*;

fn wire(phase: OwnedBackingPhase) -> OwnedBackingObservationWire {
    OwnedBackingObservationWire {
        id: Uuid::new_v4(),
        purpose_hash: [1; 32],
        generation: 1,
        phase,
        root_device: 1,
        root_inode: 2,
        namespace_device: 1,
        namespace_inode: 3,
        owner_uid: 1000,
        backing_device: 1,
        backing_inode: 4,
        birth_record_hash: [2; 32],
        inode_record_hash: [3; 32],
        journal_hash: [4; 32],
        never_format_started: matches!(
            phase,
            OwnedBackingPhase::Recorded | OwnedBackingPhase::FailedBeforeFormat
        ),
        capacity_bytes: 16_777_216,
        filesystem: (phase == OwnedBackingPhase::Ready).then(|| OwnedFilesystemBirth {
            filesystem_uuid: Uuid::new_v4(),
            block_size: 4096,
            block_count: 4096,
            clean: true,
        }),
    }
}

#[test]
fn initial_ready_requires_clean_exact_filesystem_geometry() {
    let valid = wire(OwnedBackingPhase::Ready);
    assert!(OwnedBackingObservation::try_from(valid.clone()).is_ok());
    let mut dirty = valid.clone();
    dirty.filesystem.as_mut().unwrap().clean = false;
    assert!(OwnedBackingObservation::try_from(dirty).is_err());
    let mut wrong = valid;
    wrong.filesystem.as_mut().unwrap().block_count += 1;
    assert!(OwnedBackingObservation::try_from(wrong).is_err());
}

#[test]
fn retry_proof_cannot_claim_format_started_or_unbounded_inode() {
    let mut invalid = wire(OwnedBackingPhase::FailedBeforeFormat);
    invalid.never_format_started = false;
    assert!(OwnedBackingObservation::try_from(invalid).is_err());
    let mut invalid = wire(OwnedBackingPhase::FormatIntent);
    invalid.never_format_started = true;
    assert!(OwnedBackingObservation::try_from(invalid).is_err());
    let mut invalid = wire(OwnedBackingPhase::Recorded);
    invalid.backing_inode = 0;
    assert!(OwnedBackingObservation::try_from(invalid).is_err());
    let mut invalid = wire(OwnedBackingPhase::Recorded);
    invalid.generation = u64::MAX;
    assert!(OwnedBackingObservation::try_from(invalid).is_err());
}

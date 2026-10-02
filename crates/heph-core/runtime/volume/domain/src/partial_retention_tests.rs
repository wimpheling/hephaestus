use super::*;

fn wire(phase: OwnedPartialBirthPhase) -> OwnedPartialBirthObservationWire {
    OwnedPartialBirthObservationWire {
        id: Uuid::new_v4(),
        observation_version: 1,
        volume_id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        seal_id: VolumeCreationSealId::from_uuid(Uuid::new_v4()).expect("seal"),
        original_creation: VolumeCreationIdentityWire {
            scope_id: crate::VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).expect("scope"),
            operation_id: crate::VolumeCreationOperationId::from_uuid(Uuid::new_v4()).expect("op"),
            command_id: Uuid::new_v4(),
            effect_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            resource_version: 1,
            generation: 1,
            input_hash: [5; 32],
            intent_hash: [6; 32],
        },
        provisioning_operation_id: Uuid::new_v4(),
        purpose_hash: [1; 32],
        generation: 1,
        phase,
        declared_capacity_bytes: 16 * 1024 * 1024,
        actual_length_bytes: if phase == OwnedPartialBirthPhase::ClaimedEmpty {
            0
        } else {
            16 * 1024 * 1024
        },
        owner_namespace: VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).expect("owner"),
        root_device: 1,
        root_inode: 2,
        root_owner_uid: 1000,
        namespace_device: 1,
        namespace_inode: 3,
        owner_uid: 1000,
        backing_device: 1,
        backing_inode: 4,
        birth_record_hash: [2; 32],
        inode_record_hash: [3; 32],
        journal_hash: [4; 32],
    }
}

#[test]
fn physical_cas_and_original_correlation_are_independent_checked_fences() {
    let valid = wire(OwnedPartialBirthPhase::FormatIntentIncomplete);
    let mut invalid = valid.clone();
    invalid.observation_version = 0;
    assert!(OwnedPartialBirthObservation::try_from(invalid).is_err());
    let mut invalid = valid.clone();
    invalid.original_creation.attempt_id = Uuid::nil();
    assert!(OwnedPartialBirthObservation::try_from(invalid).is_err());
    let mut invalid = valid.clone();
    invalid.provisioning_operation_id = Uuid::nil();
    assert!(OwnedPartialBirthObservation::try_from(invalid).is_err());
    let mut changed = valid;
    changed.observation_version = 2;
    changed.original_creation.generation = 7;
    let checked = OwnedPartialBirthObservation::try_from(changed).expect("bounded distinct fences");
    assert_eq!(checked.fields().observation_version, 2);
    assert_eq!(checked.fields().generation, 1);
    assert_eq!(checked.fields().original_creation.generation, 7);
}

#[test]
fn full_original_identity_comparison_rejects_changed_actor_or_input() {
    let valid = wire(OwnedPartialBirthPhase::ClaimedEmpty);
    let original = VolumeCreationIdentity::try_from(valid.original_creation.clone()).unwrap();
    let checked = OwnedPartialBirthObservation::try_from(valid).unwrap();
    assert!(checked.matches_creation(&original));
    let mut changed = original.fields().clone();
    changed.actor_id = Uuid::new_v4();
    assert!(!checked.matches_creation(&VolumeCreationIdentity::try_from(changed).unwrap()));
    let mut changed = original.fields().clone();
    changed.input_hash = [99; 32];
    assert!(!checked.matches_creation(&VolumeCreationIdentity::try_from(changed).unwrap()));
}
#[test]
fn phase_and_actual_length_are_distinct_from_declared_capacity() {
    for phase in [
        OwnedPartialBirthPhase::ClaimedEmpty,
        OwnedPartialBirthPhase::AllocatedNeverFormat,
        OwnedPartialBirthPhase::FormatIntentIncomplete,
    ] {
        let valid = wire(phase);
        assert!(OwnedPartialBirthObservation::try_from(valid.clone()).is_ok());
        let mut invalid = valid;
        invalid.actual_length_bytes = 4096;
        assert!(OwnedPartialBirthObservation::try_from(invalid).is_err());
    }
}
#[test]
fn unknown_ready_and_unbounded_or_missing_birth_are_rejected() {
    assert!(serde_json::from_str::<OwnedPartialBirthPhase>("\"ready\"").is_err());
    assert!(serde_json::from_str::<OwnedPartialBirthPhase>("\"unknown\"").is_err());
    let valid = wire(OwnedPartialBirthPhase::AllocatedNeverFormat);
    let mut missing = valid.clone();
    missing.backing_inode = 0;
    assert!(OwnedPartialBirthObservation::try_from(missing).is_err());
    let mut overflow = valid.clone();
    overflow.generation = u64::MAX;
    assert!(OwnedPartialBirthObservation::try_from(overflow).is_err());
    let mut invalid = valid;
    invalid.declared_capacity_bytes = MAX_VOLUME_CAPACITY_BYTES + 4096;
    assert!(OwnedPartialBirthObservation::try_from(invalid).is_err());
    assert!(OwnedPartialRetentionReceiptId::from_uuid(Uuid::nil()).is_err());
}
#[test]
fn observation_roundtrip_revalidates_untrusted_fields_without_authenticating_proof() {
    let original =
        OwnedPartialBirthObservation::try_from(wire(OwnedPartialBirthPhase::ClaimedEmpty))
            .expect("checked empty birth");
    let bytes = serde_json::to_vec(&original).expect("encode");
    let decoded = serde_json::from_slice::<OwnedPartialBirthObservationWire>(&bytes)
        .expect("untrusted decode");
    assert_eq!(
        OwnedPartialBirthObservation::try_from(decoded).expect("recheck"),
        original
    );
    let mut json = serde_json::to_value(&original).expect("json");
    json["filesystem"] = serde_json::json!({"clean":true});
    assert!(serde_json::from_value::<OwnedPartialBirthObservationWire>(json).is_err());
}

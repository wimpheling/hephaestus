//! Pure immutable comparison controls, NOT actor/worker/native custody proof.
use super::preparation;
use recipe_application::PreparedOwnedVolumeCreation;
use release_domain::ContentHash;
use runtime_types::VolumeId;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use volume_trait::{
    OwnedBackingPurpose, OwnedProvisioningClaim, OwnedVolumeRegistration,
    OwnedVolumeRegistrationReceipt, VolumeCreationIdentity, VolumeCreationIdentityWire,
    VolumeCreationOperationId, VolumeCreationSealId, VolumeOwnershipScopeId, VolumeRegistration,
    VolumeRootNamespaceId,
};

fn comparisons() -> (PreparedOwnedVolumeCreation, OwnedProvisioningClaim) {
    let creation = VolumeCreationIdentity::try_from(VolumeCreationIdentityWire {
        scope_id: VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).unwrap(),
        operation_id: VolumeCreationOperationId::from_uuid(Uuid::new_v4()).unwrap(),
        command_id: Uuid::new_v4(),
        effect_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        actor_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        resource_version: 2,
        generation: 1,
        input_hash: [1; 32],
        intent_hash: [2; 32],
    })
    .unwrap();
    let prepared = PreparedOwnedVolumeCreation {
        mapping_version: 1,
        claim_digest: ContentHash::from_digest([3; 32]),
        creation,
        volume_id: VolumeId::new(),
        project_id: Uuid::new_v4(),
        capacity_bytes: 16_777_216,
        filesystem_uuid: Uuid::new_v4(),
    };
    let intent = OwnedVolumeRegistration::new(
        VolumeRegistration::new(
            prepared.volume_id,
            prepared.project_id,
            prepared.capacity_bytes,
            prepared.filesystem_uuid,
        )
        .unwrap(),
        prepared.creation.clone(),
    );
    let registration_hash = Sha256::digest(intent.canonical_bytes()).into();
    let receipt = OwnedVolumeRegistrationReceipt {
        intent,
        seal_id: VolumeCreationSealId::from_uuid(Uuid::new_v4()).unwrap(),
        registration_hash,
        first_request_id: Uuid::new_v4(),
        event_id: Uuid::new_v4(),
        event_cursor: 1,
        event_aggregate_version: 1,
    };
    let purpose = OwnedBackingPurpose::new(
        receipt,
        "controlled-host".into(),
        "/controlled/volumes".into(),
        VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap(),
        1,
    )
    .unwrap();
    (
        prepared,
        OwnedProvisioningClaim {
            purpose,
            operation_id: Uuid::new_v4(),
            generation: 1,
            request_id: Uuid::new_v4(),
            event_id: Uuid::new_v4(),
        },
    )
}

#[test]
fn unacquired_guard_and_plan_do_not_prevent_original_preparation_comparison() {
    let (prepared, claim) = comparisons();
    // No filesystem, guard, native plan or permission is fabricated by this
    // comparison. Production still requires real B admission/begin/worker/probe.
    preparation(&prepared, &claim).unwrap();
}

#[test]
fn first_handover_cannot_substitute_original_preparation_without_a_saved_plan() {
    let (prepared, claim) = comparisons();
    for field in 0..6 {
        let mut changed = prepared.clone();
        match field {
            0 => changed.mapping_version = 2,
            1 => changed.volume_id = VolumeId::new(),
            2 => changed.project_id = Uuid::new_v4(),
            3 => changed.capacity_bytes += 4096,
            4 => changed.filesystem_uuid = Uuid::new_v4(),
            _ => {
                let mut creation = changed.creation.fields().clone();
                creation.input_hash = [7; 32];
                changed.creation = VolumeCreationIdentity::try_from(creation).unwrap();
            }
        }
        assert!(preparation(&changed, &claim).is_err());
    }
    preparation(&prepared, &claim).unwrap();
}

use super::*;
use crate::{
    VolumeCreationIdentity, VolumeCreationIdentityWire, VolumeCreationOperationId,
    VolumeOwnershipScopeId, VolumeRegistration,
};
use runtime_types::VolumeId;
use sha2::Digest;

fn intent() -> OwnedVolumeRegistration {
    OwnedVolumeRegistration::new(
        VolumeRegistration::new(VolumeId::new(), Uuid::new_v4(), 16_777_216, Uuid::new_v4())
            .unwrap(),
        VolumeCreationIdentity::try_from(VolumeCreationIdentityWire {
            scope_id: VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).unwrap(),
            operation_id: VolumeCreationOperationId::from_uuid(Uuid::new_v4()).unwrap(),
            command_id: Uuid::new_v4(),
            effect_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            resource_version: 1,
            generation: 1,
            input_hash: [1; 32],
            intent_hash: [2; 32],
        })
        .unwrap(),
    )
}
fn expected(intent: OwnedVolumeRegistration) -> OwnedProvisioningExpectation {
    OwnedProvisioningExpectation::new(
        intent,
        "host".into(),
        "/volumes".into(),
        VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap(),
    )
    .unwrap()
}

#[test]
fn first_operation_is_stable_versioned_and_derived_from_original_creation() {
    let registration = intent();
    let first = expected(registration.clone());
    let restart = expected(registration.clone());
    assert_eq!(first.first_operation_id(), restart.first_operation_id());
    assert_eq!(
        first.first_operation_id(),
        Uuid::new_v5(
            &registration.creation().fields().operation_id.as_uuid(),
            b"heph-owned-first-provision-v1\0"
        )
    );
    let mut changed = registration.creation().fields().clone();
    changed.operation_id = VolumeCreationOperationId::from_uuid(Uuid::new_v4()).unwrap();
    let changed = OwnedVolumeRegistration::new(
        registration.registration().clone(),
        VolumeCreationIdentity::try_from(changed).unwrap(),
    );
    assert_ne!(
        first.first_operation_id(),
        expected(changed).first_operation_id()
    );
}

#[test]
fn configured_scope_requires_bounded_canonical_labels() {
    let registration = intent();
    let namespace = VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap();
    for (host, root) in [
        ("", "/volumes"),
        ("host\n", "/volumes"),
        ("host", "volumes"),
        ("host", "/volumes/../other"),
        ("host", "/volumes//other"),
        ("host", "/volumes/./other"),
    ] {
        assert!(
            OwnedProvisioningExpectation::new(
                registration.clone(),
                host.into(),
                root.into(),
                namespace
            )
            .is_err()
        );
    }
}

#[test]
fn discovery_inputs_and_scope_cannot_accept_another_registration_receipt() {
    let first = expected(intent());
    let other = intent();
    let bytes = other.canonical_bytes();
    let receipt = OwnedVolumeRegistrationReceipt {
        intent: other,
        seal_id: crate::VolumeCreationSealId::from_uuid(Uuid::new_v4()).unwrap(),
        registration_hash: sha2::Sha256::digest(bytes).into(),
        first_request_id: Uuid::new_v4(),
        event_id: Uuid::new_v4(),
        event_cursor: 1,
        event_aggregate_version: 1,
    };
    assert!(first.purpose(receipt).is_err());
}

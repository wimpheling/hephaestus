use super::{OwnedReadyRetentionClosure, OwnedReadyRetentionClosureWire};
use runtime_types::VolumeId;
use uuid::Uuid;

fn wire() -> OwnedReadyRetentionClosureWire {
    OwnedReadyRetentionClosureWire {
        id: Uuid::new_v4(),
        volume_id: VolumeId::from_uuid(Uuid::new_v4()),
        project_id: Uuid::new_v4(),
        creation_operation_id: Uuid::new_v4(),
        provisioning_operation_id: Uuid::new_v4(),
        generation: 1,
        purpose_hash: [1; 32],
        ready_observation_id: Uuid::new_v4(),
        vm_provider_namespace: "actual-owner".into(),
        vm_host_id: "actual-host".into(),
    }
}

#[test]
fn opaque_vm_namespace_is_comparison_data_without_uuid_conversion() {
    let checked = OwnedReadyRetentionClosure::try_from(wire()).unwrap();
    assert_eq!(checked.fields().vm_provider_namespace, "actual-owner");
    assert!(Uuid::parse_str(&checked.fields().vm_provider_namespace).is_err());
    let serialized = serde_json::to_value(&checked).unwrap();
    assert_eq!(serialized["vm_provider_namespace"], "actual-owner");
}

#[test]
fn foreign_unbounded_or_invalid_pin_shapes_are_rejected() {
    for label in ["", ".", "..", "space label", "path/name", "line\nbreak"] {
        let mut altered = wire();
        altered.vm_provider_namespace = label.into();
        assert!(OwnedReadyRetentionClosure::try_from(altered).is_err());
        let mut altered = wire();
        altered.vm_host_id = label.into();
        assert!(OwnedReadyRetentionClosure::try_from(altered).is_err());
    }
    let mut altered = wire();
    altered.vm_provider_namespace = "x".repeat(129);
    assert!(OwnedReadyRetentionClosure::try_from(altered).is_err());
    let mut altered = wire();
    altered.generation = 2;
    assert!(OwnedReadyRetentionClosure::try_from(altered).is_err());
    let mut altered = wire();
    altered.ready_observation_id = Uuid::nil();
    assert!(OwnedReadyRetentionClosure::try_from(altered).is_err());
}

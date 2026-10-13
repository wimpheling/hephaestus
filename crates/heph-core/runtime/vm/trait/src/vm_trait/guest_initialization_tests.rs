use super::*;
use serde_json::json;

fn state() -> VmGuestVolume {
    VmGuestVolume::new(
        CapabilitySlotKey::parse("state").unwrap(),
        "state-disk",
        Uuid::from_u128(1),
        GuestMountPath::parse("/var/lib/hephaestus").unwrap(),
        VolumeAccessMode::ReadWrite,
    )
    .unwrap()
}

#[test]
fn default_data_metadata_never_requests_builtin_initialization() {
    let volume = state();
    assert_eq!(
        volume.initialization_purpose(),
        VmVolumeInitializationPurpose::None
    );
    assert!(!volume.frozen_requires_state());
    let wire = serde_json::to_value(&volume).unwrap();
    assert!(wire.get("initialization_purpose").is_none());
    assert!(wire.get("frozen_requires_state").is_none());
    assert_eq!(
        serde_json::from_value::<VmGuestVolume>(wire).unwrap(),
        volume
    );
}

#[test]
fn builtin_purpose_requires_exact_legacy_shape_and_explicit_frozen_marker() {
    let valid = state()
        .with_initialization_purpose(VmVolumeInitializationPurpose::BuiltinStateSQLite, true)
        .unwrap();
    assert_eq!(
        valid.initialization_purpose(),
        VmVolumeInitializationPurpose::BuiltinStateSQLite
    );
    assert!(valid.frozen_requires_state());
    let wire = serde_json::to_value(valid).unwrap();
    for (field, value) in [
        ("slot", json!("data")),
        ("guest_path", json!("/data")),
        ("access_mode", json!("read_only")),
        ("frozen_requires_state", json!(false)),
        ("initialization_purpose", json!("unknown")),
        ("initialization_purpose", json!("none")),
    ] {
        let mut invalid = wire.clone();
        invalid[field] = value;
        assert!(
            serde_json::from_value::<VmGuestVolume>(invalid).is_err(),
            "{field}"
        );
    }
    let mut missing = wire.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("frozen_requires_state");
    assert!(serde_json::from_value::<VmGuestVolume>(missing).is_err());
    assert_eq!(
        serde_json::from_value::<VmGuestVolume>(wire)
            .unwrap()
            .guest_path()
            .as_str(),
        "/var/lib/hephaestus"
    );
    assert!(
        state()
            .with_initialization_purpose(VmVolumeInitializationPurpose::BuiltinStateSQLite, false)
            .is_err()
    );
    assert!(
        state()
            .with_initialization_purpose(VmVolumeInitializationPurpose::None, true)
            .is_err()
    );
}

#[test]
fn multiple_builtin_initializers_cannot_alias_the_same_mount() {
    let first = state()
        .with_initialization_purpose(VmVolumeInitializationPurpose::BuiltinStateSQLite, true)
        .unwrap();
    let second = VmGuestVolume::new(
        CapabilitySlotKey::parse("state").unwrap(),
        "another-disk",
        Uuid::from_u128(2),
        GuestMountPath::parse("/var/lib/hephaestus").unwrap(),
        VolumeAccessMode::ReadWrite,
    )
    .unwrap()
    .with_initialization_purpose(VmVolumeInitializationPurpose::BuiltinStateSQLite, true)
    .unwrap();
    assert!(validate_guest_volume_set(&[first, second]).is_err());
}

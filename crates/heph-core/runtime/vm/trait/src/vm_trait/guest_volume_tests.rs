use super::*;
use serde_json::json;

fn volume(name: &str, disk: &str, id: u128, path: &str) -> VmGuestVolume {
    VmGuestVolume::new(
        CapabilitySlotKey::parse(name).unwrap(),
        disk,
        Uuid::from_u128(id),
        GuestMountPath::parse(path).unwrap(),
        VolumeAccessMode::ReadOnly,
    )
    .unwrap()
}

#[test]
fn checked_wire_rejects_unknown_fields_nil_uuid_and_disk_escape() {
    let valid = serde_json::to_value(volume("data", "data-disk", 1, "/data")).unwrap();
    let parsed: VmGuestVolume = serde_json::from_value(valid.clone()).unwrap();
    assert_eq!(parsed.filesystem_uuid(), Uuid::from_u128(1));
    for (field, value) in [
        ("host_path", json!("/host/secret")),
        ("filesystem_uuid", json!(Uuid::nil())),
        ("disk_id", json!("../disk")),
        ("guest_path", json!("/workspace/repo")),
        ("access_mode", json!("write_only")),
        ("slot", json!("BadName")),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(
            serde_json::from_value::<VmGuestVolume>(invalid).is_err(),
            "{field}"
        );
    }
}

#[test]
fn sets_reject_every_identity_alias_and_path_overlap() {
    let first = volume("data", "data-disk", 1, "/data");
    for other in [
        volume("data", "other-disk", 2, "/other"),
        volume("other", "data-disk", 2, "/other"),
        volume("other", "other-disk", 1, "/other"),
        volume("other", "other-disk", 2, "/data/child"),
        volume("other", "other-disk", 2, "/data"),
    ] {
        assert!(validate_guest_volume_set(&[first.clone(), other]).is_err());
    }
    validate_guest_volume_set(&[
        first.clone(),
        volume("other", "other-disk", 2, "/data-other"),
    ])
    .unwrap();
    assert!(validate_guest_volume_set(&vec![first; MAX_VOLUME_SLOTS + 1]).is_err());
}

#[test]
fn platform_paths_are_canonical_and_root_masks_every_attachment() {
    let volumes = [volume("data", "data-disk", 1, "/data")];
    for path in [
        "/",
        "/data",
        "/data/child",
        "/data//child",
        "/data/./child",
        "/data/child/",
        "/other/../data",
        "relative",
    ] {
        assert!(
            validate_guest_volume_mounts(&volumes, &[path]).is_err(),
            "{path}"
        );
    }
    validate_guest_volume_mounts(&volumes, &["/release", "/workspace/repo", "/data-other"])
        .unwrap();
}

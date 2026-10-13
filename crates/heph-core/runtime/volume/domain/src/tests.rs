use capability_domain::CapabilitySlotKey;
use runtime_types::VolumeId;

use crate::{
    GuestMountPath, MAX_GUEST_MOUNT_PATH_BYTES, MAX_VOLUME_CAPACITY_BYTES, MAX_VOLUME_SLOTS,
    VolumeAccessMode, VolumeContractError, VolumeSlotBinding, VolumeSlotDeclaration,
    validate_volume_bindings, validate_volume_slots,
};

fn declaration(key: &str, path: &str, required: bool) -> VolumeSlotDeclaration {
    VolumeSlotDeclaration::new(
        CapabilitySlotKey::parse(key).expect("valid slot key"),
        GuestMountPath::parse(path).expect("valid mount path"),
        VolumeAccessMode::ReadWrite,
        required,
        1024,
    )
    .expect("valid capacity")
}

fn binding(key: &str, volume_id: VolumeId, mode: VolumeAccessMode) -> VolumeSlotBinding {
    VolumeSlotBinding::new(
        CapabilitySlotKey::parse(key).expect("valid slot key"),
        volume_id,
        mode,
    )
}

#[test]
fn rejects_hostile_and_protected_mount_paths() {
    for path in [
        "",
        "/",
        "relative",
        "../data",
        "/data/",
        "//data",
        "/data//sqlite",
        "/data/./sqlite",
        "/data/../sqlite",
        "/data\0sqlite",
        "/data\nsqlite",
        "/data\u{7f}sqlite",
        "/data\\sqlite",
        "/proc",
        "/proc/mounts",
        "/sys",
        "/sys/kernel",
        "/dev",
        "/dev/shm/sqlite",
        "/run",
        "/run/sqlite",
        "/release",
        "/release/data",
        "/workspace",
        "/workspace/data",
    ] {
        assert_eq!(
            GuestMountPath::parse(path),
            Err(VolumeContractError::InvalidGuestMountPath),
            "accepted hostile path {path:?}"
        );
        assert!(serde_json::from_value::<GuestMountPath>(serde_json::json!(path)).is_err());
    }
    let oversized = format!("/{}", "a".repeat(MAX_GUEST_MOUNT_PATH_BYTES));
    assert!(GuestMountPath::parse(oversized).is_err());
}

#[test]
fn accepts_canonical_paths_with_component_boundaries_and_legacy_mount() {
    for path in [
        "/data/sqlite",
        "/var/lib/hephaestus",
        "/process",
        "/sysdata",
        "/device",
        "/runner",
        "/release-data",
        "/workspace-data",
    ] {
        let validated = GuestMountPath::parse(path).expect("controlled path");
        assert_eq!(validated.as_str(), path);
        let encoded = serde_json::to_value(&validated).expect("serialize path");
        assert_eq!(
            serde_json::from_value::<GuestMountPath>(encoded).expect("round trip"),
            validated
        );
    }
}

#[test]
fn capacity_is_positive_bounded_and_rechecked_by_deserialization() {
    let base = declaration("data", "/data", true);
    for capacity in [0, MAX_VOLUME_CAPACITY_BYTES + 1, u64::MAX] {
        assert_eq!(
            VolumeSlotDeclaration::new(
                base.slot().clone(),
                base.guest_path().clone(),
                base.access_mode(),
                true,
                capacity
            ),
            Err(VolumeContractError::InvalidCapacity {
                maximum: MAX_VOLUME_CAPACITY_BYTES
            })
        );
        let mut encoded = serde_json::to_value(&base).expect("serialize declaration");
        encoded["minimum_capacity_bytes"] = capacity.into();
        assert!(serde_json::from_value::<VolumeSlotDeclaration>(encoded).is_err());
    }
    for capacity in [1, MAX_VOLUME_CAPACITY_BYTES] {
        let slot = VolumeSlotDeclaration::new(
            base.slot().clone(),
            base.guest_path().clone(),
            base.access_mode(),
            true,
            capacity,
        )
        .expect("inclusive capacity bound");
        assert_eq!(slot.minimum_capacity_bytes(), capacity);
        let encoded = serde_json::to_value(&slot).expect("serialize declaration");
        assert_eq!(
            serde_json::from_value::<VolumeSlotDeclaration>(encoded).expect("round trip"),
            slot
        );
    }
}

#[test]
fn declaration_sets_reject_duplicate_keys_and_overlapping_mounts() {
    let first = declaration("data", "/data", true);
    assert!(matches!(
        validate_volume_slots(&[first.clone(), declaration("data", "/other", false)]),
        Err(VolumeContractError::DuplicateSlot(_))
    ));
    for second in ["/data", "/data/sqlite"] {
        assert_eq!(
            validate_volume_slots(&[first.clone(), declaration("other", second, false)]),
            Err(VolumeContractError::OverlappingMountPaths)
        );
    }
    assert_eq!(
        validate_volume_slots(&[declaration("inner", "/data/sqlite", true), first.clone()]),
        Err(VolumeContractError::OverlappingMountPaths)
    );
    assert!(validate_volume_slots(&[first, declaration("other", "/database", false)]).is_ok());
}

#[test]
fn required_optional_and_unexpected_bindings_are_explicit() {
    let slots = [
        declaration("data", "/data", true),
        declaration("cache", "/cache", false),
    ];
    assert!(matches!(
        validate_volume_bindings(&slots, &[]),
        Err(VolumeContractError::MissingRequiredBinding(_))
    ));
    let selected = binding("data", VolumeId::new(), VolumeAccessMode::ReadWrite);
    assert!(validate_volume_bindings(&slots, &[selected.clone()]).is_ok());
    assert!(validate_volume_bindings(&[], &[]).is_ok());
    assert!(validate_volume_bindings(&[declaration("cache", "/cache", false)], &[]).is_ok());
    assert!(matches!(
        validate_volume_bindings(
            &slots,
            &[
                selected,
                binding("unknown", VolumeId::new(), VolumeAccessMode::ReadWrite)
            ]
        ),
        Err(VolumeContractError::UnexpectedBinding(_))
    ));
}

#[test]
fn modes_match_exactly_in_both_directions() {
    for (declared, selected) in [
        (VolumeAccessMode::ReadOnly, VolumeAccessMode::ReadWrite),
        (VolumeAccessMode::ReadWrite, VolumeAccessMode::ReadOnly),
    ] {
        let slot = VolumeSlotDeclaration::new(
            CapabilitySlotKey::parse("data").expect("slot"),
            GuestMountPath::parse("/data").expect("path"),
            declared,
            true,
            1,
        )
        .expect("declaration");
        assert!(matches!(
            validate_volume_bindings(
                &[slot.clone()],
                &[binding("data", VolumeId::new(), selected)]
            ),
            Err(VolumeContractError::AccessModeMismatch(_))
        ));
        assert!(
            validate_volume_bindings(&[slot], &[binding("data", VolumeId::new(), declared)])
                .is_ok()
        );
    }
}

#[test]
fn rejects_duplicate_bindings_and_resource_reuse_even_when_read_only() {
    let slots = [
        declaration("data", "/data", true),
        declaration("cache", "/cache", false),
    ];
    let selected = binding("data", VolumeId::new(), VolumeAccessMode::ReadWrite);
    assert!(matches!(
        validate_volume_bindings(&slots, &[selected.clone(), selected.clone()]),
        Err(VolumeContractError::DuplicateSlot(_))
    ));
    assert_eq!(
        validate_volume_bindings(
            &slots,
            &[
                selected.clone(),
                binding("cache", selected.volume_id(), VolumeAccessMode::ReadWrite)
            ]
        ),
        Err(VolumeContractError::ReusedVolume(selected.volume_id()))
    );
    let read_only_slots = slots
        .iter()
        .map(|slot| {
            VolumeSlotDeclaration::new(
                slot.slot().clone(),
                slot.guest_path().clone(),
                VolumeAccessMode::ReadOnly,
                slot.required(),
                1,
            )
            .expect("read-only declaration")
        })
        .collect::<Vec<_>>();
    let volume_id = VolumeId::new();
    assert_eq!(
        validate_volume_bindings(
            &read_only_slots,
            &[
                binding("data", volume_id, VolumeAccessMode::ReadOnly),
                binding("cache", volume_id, VolumeAccessMode::ReadOnly)
            ]
        ),
        Err(VolumeContractError::ReusedVolume(volume_id))
    );
}

#[test]
fn complete_sets_have_explicit_count_bounds() {
    let slots = (0..MAX_VOLUME_SLOTS)
        .map(|index| declaration(&format!("data_{index}"), &format!("/data_{index}"), false))
        .collect::<Vec<_>>();
    assert!(validate_volume_slots(&slots).is_ok());
    let mut excessive = slots.clone();
    excessive.push(declaration("extra", "/extra", false));
    assert_eq!(
        validate_volume_slots(&excessive),
        Err(VolumeContractError::TooManySlots {
            maximum: MAX_VOLUME_SLOTS
        })
    );
    let bindings = (0..=MAX_VOLUME_SLOTS)
        .map(|index| {
            binding(
                &format!("data_{index}"),
                VolumeId::new(),
                VolumeAccessMode::ReadWrite,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        validate_volume_bindings(&slots, &bindings),
        Err(VolumeContractError::TooManySlots {
            maximum: MAX_VOLUME_SLOTS
        })
    );
}

use capability_domain::CapabilitySlotKey;

use crate::{
    GuestMountPath, LEGACY_STATE_VOLUME_GUEST_PATH, LEGACY_STATE_VOLUME_SLOT, VolumeAccessMode,
    VolumeContractError, VolumeSlotDeclaration, effective_volume_slots,
};

fn explicit(slot: &str, path: &str) -> VolumeSlotDeclaration {
    VolumeSlotDeclaration::new(
        CapabilitySlotKey::parse(slot).expect("slot"),
        GuestMountPath::parse(path).expect("path"),
        VolumeAccessMode::ReadOnly,
        false,
        64,
    )
    .expect("declaration")
}

#[test]
fn legacy_slot_preserves_mode_path_and_undeclared_capacity_compatibility() {
    assert!(
        effective_volume_slots(&[], false)
            .expect("stateless")
            .is_empty()
    );
    let slots = effective_volume_slots(&[], true).expect("legacy declaration");
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].slot().as_str(), LEGACY_STATE_VOLUME_SLOT);
    assert_eq!(
        slots[0].guest_path().as_str(),
        LEGACY_STATE_VOLUME_GUEST_PATH
    );
    assert_eq!(slots[0].access_mode(), VolumeAccessMode::ReadWrite);
    assert!(slots[0].required());
    assert_eq!(slots[0].minimum_capacity_bytes(), 1);
}

#[test]
fn effective_catalog_rejects_legacy_name_and_mount_collisions() {
    assert!(matches!(
        effective_volume_slots(&[explicit("state", "/data")], true),
        Err(VolumeContractError::DuplicateSlot(_))
    ));
    for path in [
        "/var/lib/hephaestus",
        "/var/lib/hephaestus/sqlite",
        "/var/lib",
    ] {
        assert_eq!(
            effective_volume_slots(&[explicit("data", path)], true),
            Err(VolumeContractError::OverlappingMountPaths)
        );
    }
}

#[test]
fn effective_catalog_sorts_a_clone_without_mutating_authored_slots() {
    let slots = [explicit("zeta", "/zeta"), explicit("alpha", "/alpha")];
    let effective = effective_volume_slots(&slots, true).expect("compatible catalog");
    assert_eq!(
        effective
            .iter()
            .map(|slot| slot.slot().as_str())
            .collect::<Vec<_>>(),
        ["alpha", "state", "zeta"]
    );
    assert_eq!(slots[0].slot().as_str(), "zeta");
    assert!(!effective[0].required());
    assert_eq!(effective[0].access_mode(), VolumeAccessMode::ReadOnly);
}

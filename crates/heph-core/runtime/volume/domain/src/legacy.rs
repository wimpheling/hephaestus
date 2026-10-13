use capability_domain::CapabilitySlotKey;

use crate::{
    GuestMountPath, VolumeAccessMode, VolumeContractError, VolumeSlotDeclaration,
    validate_volume_slots,
};

/// Symbolic slot used to represent the historical instance-state volume.
pub const LEGACY_STATE_VOLUME_SLOT: &str = "state";
/// Controlled guest mount path of the historical instance-state volume.
pub const LEGACY_STATE_VOLUME_GUEST_PATH: &str = "/var/lib/hephaestus";

/// Returns validated explicit slots and the enabled historical state slot.
///
/// The compatibility slot requires writable attachment but declares only a
/// one-byte minimum: old releases had no release-owned capacity requirement.
/// Provider limits and existing volume capacity continue to apply. The result
/// never grants authority or changes a stored release's identity or hashes.
///
/// # Errors
///
/// Rejects duplicate slot names, mount overlaps, and excessive combined count.
pub fn effective_volume_slots(
    slots: &[VolumeSlotDeclaration],
    requires_state: bool,
) -> Result<Vec<VolumeSlotDeclaration>, VolumeContractError> {
    let mut effective = slots.to_vec();
    if requires_state {
        let slot = CapabilitySlotKey::parse(LEGACY_STATE_VOLUME_SLOT)
            .map_err(|_| VolumeContractError::InvalidLegacyDeclaration)?;
        effective.push(VolumeSlotDeclaration::new(
            slot,
            GuestMountPath::parse(LEGACY_STATE_VOLUME_GUEST_PATH)?,
            VolumeAccessMode::ReadWrite,
            true,
            1,
        )?);
    }
    validate_volume_slots(&effective)?;
    effective.sort_unstable_by(|left, right| left.slot().cmp(right.slot()));
    Ok(effective)
}

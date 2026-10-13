use std::collections::{BTreeMap, BTreeSet};

use crate::{
    MAX_VOLUME_SLOTS, VolumeContractError, VolumeSlotBinding, VolumeSlotDeclaration,
    path::paths_overlap,
};

/// Validates the bounded complete slot set before any provider effects.
///
/// # Errors
///
/// Rejects excessive slot counts, duplicate keys, and equal or nested mounts.
pub fn validate_volume_slots(slots: &[VolumeSlotDeclaration]) -> Result<(), VolumeContractError> {
    check_count(slots.len())?;
    let mut names = BTreeSet::new();
    for (index, slot) in slots.iter().enumerate() {
        if !names.insert(slot.slot()) {
            return Err(VolumeContractError::DuplicateSlot(slot.slot().clone()));
        }
        if slots[..index].iter().any(|previous| {
            paths_overlap(previous.guest_path().as_str(), slot.guest_path().as_str())
        }) {
            return Err(VolumeContractError::OverlappingMountPaths);
        }
    }
    Ok(())
}

/// Validates exact slot bindings without inferring authorization.
///
/// # Errors
///
/// Rejects invalid declarations, excessive or duplicate bindings, unexpected
/// slots, missing required slots, mode mismatches, and reused volume IDs.
pub fn validate_volume_bindings(
    slots: &[VolumeSlotDeclaration],
    bindings: &[VolumeSlotBinding],
) -> Result<(), VolumeContractError> {
    validate_volume_slots(slots)?;
    check_count(bindings.len())?;
    let declarations = slots
        .iter()
        .map(|slot| (slot.slot(), slot))
        .collect::<BTreeMap<_, _>>();
    let mut names = BTreeSet::new();
    let mut volumes = BTreeSet::new();
    for binding in bindings {
        if !names.insert(binding.slot()) {
            return Err(VolumeContractError::DuplicateSlot(binding.slot().clone()));
        }
        let declaration = declarations
            .get(binding.slot())
            .ok_or_else(|| VolumeContractError::UnexpectedBinding(binding.slot().clone()))?;
        if binding.access_mode() != declaration.access_mode() {
            return Err(VolumeContractError::AccessModeMismatch(
                binding.slot().clone(),
            ));
        }
        if !volumes.insert(binding.volume_id()) {
            return Err(VolumeContractError::ReusedVolume(binding.volume_id()));
        }
    }
    for declaration in slots {
        if declaration.required() && !names.contains(declaration.slot()) {
            return Err(VolumeContractError::MissingRequiredBinding(
                declaration.slot().clone(),
            ));
        }
    }
    Ok(())
}

const fn check_count(count: usize) -> Result<(), VolumeContractError> {
    if count > MAX_VOLUME_SLOTS {
        return Err(VolumeContractError::TooManySlots {
            maximum: MAX_VOLUME_SLOTS,
        });
    }
    Ok(())
}

use std::collections::{BTreeMap, BTreeSet};

use capability_domain::{CapabilityOperation, CapabilityResourceKind, CapabilitySlotKey};
use volume_domain::validate_volume_slots;

use crate::{
    InstanceResource, RecipeError, ReleaseCatalogEntry, ResolvedResource, catalog::slot_path,
    errors::invalid,
};

pub fn validate_bindings(
    instance: &InstanceResource,
    catalog: &ReleaseCatalogEntry,
    resources: &BTreeMap<CapabilitySlotKey, ResolvedResource>,
) -> Result<(), RecipeError> {
    validate_volume_slots(&catalog.volume_slots)
        .map_err(|_| invalid("invalid_release_volume_slots", instance.name.as_str()))?;
    let declarations = catalog
        .volume_slots
        .iter()
        .map(|slot| (slot.slot(), slot))
        .collect::<BTreeMap<_, _>>();
    let bindings = instance
        .volume_bindings
        .iter()
        .map(|binding| (&binding.slot, binding))
        .collect::<BTreeMap<_, _>>();
    for binding in &instance.volume_bindings {
        let path = slot_path(&instance.name, &binding.slot);
        let slot = declarations
            .get(&binding.slot)
            .ok_or_else(|| invalid("undeclared_volume_slot", &path))?;
        if binding.guest_path != *slot.guest_path() {
            return Err(invalid("volume_mount_mismatch", path));
        }
        if binding.access_mode != slot.access_mode() {
            return Err(invalid("volume_access_mismatch", path));
        }
        let Some(ResolvedResource::Volume(volume)) = resources.get(&binding.resource) else {
            return Err(invalid("unknown_volume_resource", path));
        };
        if volume.capacity_bytes < slot.minimum_capacity_bytes() {
            return Err(invalid("volume_capacity_below_release_minimum", path));
        }
    }
    for slot in &catalog.volume_slots {
        if slot.required() && !bindings.contains_key(slot.slot()) {
            return Err(invalid(
                "required_volume_slot_missing",
                slot_path(&instance.name, slot.slot()),
            ));
        }
    }
    let mut capability_slots = BTreeSet::new();
    for requirement in &catalog.capability_requirements {
        if !capability_slots.insert(requirement.slot()) {
            return Err(invalid(
                "duplicate_release_capability_slot",
                instance.name.as_str(),
            ));
        }
        if requirement.resource_kind() != CapabilityResourceKind::StateVolume {
            if requirement.slot_required() {
                return Err(invalid(
                    "unsupported_required_capability_slot",
                    instance.name.as_str(),
                ));
            }
            continue;
        }
        if !declarations.contains_key(requirement.slot()) {
            return Err(invalid(
                "capability_volume_slot_undeclared",
                instance.name.as_str(),
            ));
        }
        if requirement.required_operations().any(|operation| {
            !matches!(
                operation,
                CapabilityOperation::Inspect | CapabilityOperation::Attach
            )
        }) {
            return Err(invalid(
                "unsupported_required_volume_operation",
                instance.name.as_str(),
            ));
        }
        if requirement.slot_required() && !bindings.contains_key(requirement.slot()) {
            return Err(invalid(
                "required_capability_binding_missing",
                instance.name.as_str(),
            ));
        }
    }
    Ok(())
}

//! Checks frozen launch/catalog evidence; this does not authorize an invocation.

use control_plane_postgres::run::VmLaunchContract;
use volume_domain::{
    MAX_VOLUME_SLOTS, RunVolumeSelections, VolumeSelectionOrigin, VolumeSlotDeclaration,
    effective_volume_slots,
};

use super::{Run, VmError, invalid_spec, vm_factory_error};

pub fn validate(
    run: &Run,
    stored: &VmLaunchContract,
    selections: &RunVolumeSelections,
) -> Result<(), VmError> {
    validate_identity(run, stored, selections)?;
    let authored = authored_slots(stored)?;
    let declarations =
        effective_volume_slots(&authored, stored.requires_state).map_err(vm_factory_error)?;
    match stored.volume_mode.as_str() {
        "named" if stored.state_volume_id.is_none() => {}
        "legacy" if authored.is_empty() => {}
        _ => {
            return Err(volume_error(
                "instance volume mode contradicts its catalog or pointer",
            ));
        }
    }
    for selected in selections.selections() {
        let declared = declarations
            .iter()
            .find(|declared| declared.slot() == selected.declaration().slot())
            .ok_or_else(|| volume_error("selected slot is absent from the frozen release"))?;
        if declared != selected.declaration()
            || selected
                .scope()
                .release_contract_hash()
                .as_bytes()
                .as_slice()
                != stored.runtime_contract_hash.as_slice()
        {
            return Err(volume_error(
                "selected declaration or release hash was substituted",
            ));
        }
        let expected_origin = expected_origin(stored, &authored, selected)?;
        if selected.origin() != expected_origin {
            return Err(volume_error(
                "selected initialization provenance was substituted",
            ));
        }
    }
    if declarations.iter().any(|declared| {
        declared.required()
            && !selections
                .selections()
                .iter()
                .any(|selected| selected.declaration().slot() == declared.slot())
    }) {
        return Err(volume_error("a required frozen slot is missing"));
    }
    Ok(())
}

fn expected_origin(
    stored: &VmLaunchContract,
    authored: &[VolumeSlotDeclaration],
    selected: &volume_domain::RunVolumeSelection,
) -> Result<VolumeSelectionOrigin, VmError> {
    if stored.volume_mode == "legacy" {
        if stored.state_volume_id != Some(selected.scope().volume_id().as_uuid()) {
            return Err(volume_error(
                "legacy selection differs from its stored origin pointer",
            ));
        }
        Ok(VolumeSelectionOrigin::LegacyOrigin)
    } else if authored
        .iter()
        .any(|slot| slot.slot() == selected.declaration().slot())
    {
        Ok(VolumeSelectionOrigin::Explicit)
    } else {
        Ok(VolumeSelectionOrigin::LegacyDeclaration)
    }
}

fn validate_identity(
    run: &Run,
    stored: &VmLaunchContract,
    selections: &RunVolumeSelections,
) -> Result<(), VmError> {
    let identity = selections.identity();
    let stored_pins = [
        stored.run_id,
        stored.instance_id,
        stored.instance_revision_id,
        stored.release_id,
        stored.release_agent_id,
    ];
    let run_pins = [
        run.id.as_uuid(),
        run.instance_id.as_uuid(),
        run.instance_revision_id.as_uuid(),
        run.release_id.as_uuid(),
        run.release_agent_id.as_uuid(),
    ];
    let selected_pins = [
        identity.run_id().as_uuid(),
        identity.instance_id().as_uuid(),
        identity.revision_id().as_uuid(),
        identity.release_id().as_uuid(),
        identity.release_agent_id().as_uuid(),
    ];
    if stored_pins != run_pins
        || stored_pins != selected_pins
        || identity.project_id() != stored.project_id
        || stored.runtime_contract_hash.len() != 32
    {
        return Err(volume_error(
            "run, selected consumer or released pins do not match storage",
        ));
    }
    Ok(())
}

fn authored_slots(stored: &VmLaunchContract) -> Result<Vec<VolumeSlotDeclaration>, VmError> {
    if !stored.runtime_contract.is_object()
        || stored
            .runtime_contract
            .get("requires_state")
            .is_some_and(|value| value.as_bool() != Some(stored.requires_state))
    {
        return Err(volume_error(
            "frozen state requirement contradicts the runtime contract",
        ));
    }
    let Some(value) = stored.runtime_contract.get("volume_slots") else {
        return Ok(Vec::new());
    };
    if value
        .as_array()
        .is_none_or(|slots| slots.len() > MAX_VOLUME_SLOTS)
    {
        return Err(volume_error("frozen volume catalog is not a bounded array"));
    }
    serde_json::from_value(value.clone()).map_err(vm_factory_error)
}

fn volume_error(reason: &str) -> VmError {
    invalid_spec("volume_slots", reason)
}

#[cfg(test)]
#[path = "volumes_tests.rs"]
mod tests;

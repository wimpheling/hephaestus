//! Pure projection of checked frozen inputs to the existing107 import command.

use recipe_application::PreparedInstanceImport;
use release_service::{ImportAgentWithVolumes, VolumeSlotSelection};

/// Converts every frozen field without resolving policies, catalogs or identities.
///
/// No authority is conveyed. Command keys and grant IDs remain the exact stored
/// values; the operation ID and effect attempt ID are not import command keys.
#[must_use]
pub fn to_import_command(prepared: &PreparedInstanceImport) -> ImportAgentWithVolumes {
    let pin = prepared.release_pin();
    ImportAgentWithVolumes {
        command_key: prepared.command_key(),
        instance_id: release_domain::AgentInstanceId::from_uuid(prepared.instance_id().as_uuid()),
        revision_id: release_domain::AgentInstanceRevisionId::from_uuid(
            prepared.revision_id().as_uuid(),
        ),
        release_id: pin.release_id,
        release_agent_id: pin.release_agent_id,
        project_id: prepared.project_id(),
        name: prepared.name().clone(),
        parameters: prepared.parameters().clone(),
        selected_policy: prepared.selected_policy().clone(),
        platform_policy: prepared.platform_policy().clone(),
        platform_policy_version: prepared.platform_policy_version().to_owned(),
        volumes: prepared
            .volumes()
            .iter()
            .map(|selection| VolumeSlotSelection {
                slot: selection.slot().clone(),
                guest_path: selection.guest_path().clone(),
                access_mode: selection.access_mode(),
                volume_id: selection.volume_id(),
                grant_id: selection.grant_id(),
            })
            .collect(),
    }
}

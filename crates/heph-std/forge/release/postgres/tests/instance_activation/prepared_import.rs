use recipe_application::PreparedInstanceImport;
use release_service::{ImportAgentWithVolumes, VolumeSlotSelection};

pub fn command(prepared: &PreparedInstanceImport) -> ImportAgentWithVolumes {
    let pin = prepared.release_pin();
    ImportAgentWithVolumes {
        command_key: prepared.command_key(),
        instance_id: prepared.instance_id(),
        revision_id: prepared.revision_id(),
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

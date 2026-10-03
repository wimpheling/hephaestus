use agent_config::AgentConfig;
use capability_domain::CapabilitySlotKey;
use release_domain::{RuntimePolicy, VolumeSlotDeclaration, effective_volume_slots};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{ArtifactPath, ReleaseServiceError};

pub fn build(
    config: &AgentConfig,
    guest_image_reference: &str,
    policy: &RuntimePolicy,
) -> Result<(Value, [u8; 32]), ReleaseServiceError> {
    config
        .effective_volume_slots()
        .map_err(|_| ReleaseServiceError::InvalidStoredData)?;
    let executable = ArtifactPath::parse(config.guest.command.clone())?;
    let working_directory = ArtifactPath::parse(config.guest.working_directory.clone())?;
    let mut contract = json!({
        "executable": executable,
        "arguments": config.guest.arguments,
        "working_directory": working_directory,
        "image_reference": guest_image_reference,
        "requires_state": config.state_volume.enabled,
        "publication_mode": config.publication.mode,
        "publication_repository_slot": config.publication.repository_slot
            .as_ref().map(CapabilitySlotKey::as_str),
        "policy_ceiling": policy,
        "workspace": {
            "source": "/workspace/repo", "work": "/workspace/work",
            "release": "/release", "state": "/var/lib/hephaestus",
            "parameters": "/run/hephaestus/parameters.json"
        }
    });
    // Authored intent only: lifting the legacy slot would change frozen bytes.
    if !config.volume_slots.is_empty() {
        let mut slots = config.volume_slots.clone();
        slots.sort_by(|left, right| left.slot().cmp(right.slot()));
        contract["volume_slots"] = serde_json::to_value(slots)?;
    }
    let bytes = serde_json::to_vec(&contract)?;
    Ok((contract, Sha256::digest(bytes).into()))
}

pub fn require_legacy_import(
    contract: &Value,
    requires_state: bool,
) -> Result<(), ReleaseServiceError> {
    if !contract.is_object() {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    if contract
        .get("requires_state")
        .is_some_and(|value| value.as_bool() != Some(requires_state))
    {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    let authored: Vec<VolumeSlotDeclaration> = contract
        .get("volume_slots")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| ReleaseServiceError::InvalidStoredData)?
        .unwrap_or_default();
    effective_volume_slots(&authored, requires_state)
        .map_err(|_| ReleaseServiceError::InvalidStoredData)?;
    // Legacy import cannot materialize the new immutable typed bindings yet.
    if !authored.is_empty() {
        return Err(ReleaseServiceError::CapabilityResourceUnavailable);
    }
    Ok(())
}

#[cfg(test)]
#[path = "release_runtime_contract_tests.rs"]
mod tests;

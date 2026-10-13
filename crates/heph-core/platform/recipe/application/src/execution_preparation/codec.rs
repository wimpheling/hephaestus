use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use identity_domain::{RequestId, UserId};
use recipe_domain::ReleasePin;
use release_domain::{
    ContentHash, InstanceName, ParameterName, ParameterValue, ReleaseCommandKey, RuntimePolicy,
};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, VolumeId};
use serde::Deserialize;
use uuid::Uuid;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeMountGrantId};

use crate::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentOperation,
    PlatformPolicyObservation, SourceObservation,
};

use super::{
    DeploymentExecutionPreparation, DeploymentExecutionProfile, MAX_EXECUTION_PREPARATION_BYTES,
    mapping,
};

// All wire types are private and untrusted. The full canonical re-encoding
// comparison checks every field, including fields not used to reconstruct input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparationWire {
    version: u32,
    deployment_id: Uuid,
    project_id: ProjectId,
    intent_hash: ContentHash,
    command: CommandWire,
    profile: DeploymentExecutionProfile,
    platform: PlatformWire,
    instances: Vec<ImportWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandWire {
    id: Uuid,
    actor_id: UserId,
    idempotency_id: RequestId,
    operation: DeploymentOperation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformWire {
    policy: RuntimePolicy,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceWire {
    pin: ReleasePin,
    runtime_contract_hash: ContentHash,
    image: String,
    selected_policy: RuntimePolicy,
    platform: PlatformWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportWire {
    resource: CapabilitySlotKey,
    operation_id: Uuid,
    command_key: ReleaseCommandKey,
    instance_id: AgentInstanceId,
    revision_id: AgentInstanceRevisionId,
    project_id: ProjectId,
    name: InstanceName,
    parameters: BTreeMap<ParameterName, ParameterValue>,
    source: SourceWire,
    volumes: Vec<VolumeWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeWire {
    slot: CapabilitySlotKey,
    guest_path: GuestMountPath,
    access_mode: VolumeAccessMode,
    volume_id: VolumeId,
    grant_id: VolumeMountGrantId,
}

impl PlatformWire {
    fn checked(self) -> Result<PlatformPolicyObservation, DeploymentError> {
        PlatformPolicyObservation::new(self.policy, self.version)
    }
}

impl SourceWire {
    fn checked(&self) -> Result<SourceObservation, DeploymentError> {
        SourceObservation::new(
            self.pin,
            self.runtime_contract_hash,
            &self.image,
            self.selected_policy.clone(),
            PlatformPolicyObservation::new(
                self.platform.policy.clone(),
                self.platform.version.clone(),
            )?,
        )
    }
}

pub fn reconstruct(
    intent: &DeploymentIntent,
    command: CommandIdentity,
    bytes: &[u8],
    expected_hash: ContentHash,
) -> Result<DeploymentExecutionPreparation, DeploymentError> {
    if bytes.is_empty()
        || bytes.len() > MAX_EXECUTION_PREPARATION_BYTES
        || ContentHash::digest(bytes) != expected_hash
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let wire: PreparationWire =
        serde_json::from_slice(bytes).map_err(|_| DeploymentError::IntentMismatch)?;
    if wire.version != 1
        || wire.deployment_id != intent.id().as_uuid()
        || wire.project_id != intent.project_id()
        || wire.intent_hash != intent.input_hash()
        || wire.command.id != command.id().as_uuid()
        || wire.command.actor_id != command.actor_id()
        || wire.command.idempotency_id != command.idempotency_id()
        || wire.command.operation != command.operation()
        || wire.instances.len() > recipe_domain::MAX_RECIPE_ITEMS
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let platform = wire.platform.checked()?;
    let mut sources = BTreeMap::new();
    for instance in &wire.instances {
        check_wire_bounds(instance)?;
    }
    for instance in &wire.instances {
        let observation = instance.source.checked()?;
        let key = mapping::pin_key(observation.pin());
        if sources
            .get(&key)
            .is_some_and(|original| original != &observation)
        {
            return Err(DeploymentError::IntentMismatch);
        }
        sources.insert(key, observation);
    }
    let prepared = DeploymentExecutionPreparation::new(
        intent,
        command,
        wire.profile,
        platform,
        &sources.into_values().collect::<Vec<_>>(),
    )?;
    if wire.instances.len() != prepared.instances().len()
        || !wire
            .instances
            .iter()
            .zip(prepared.instances())
            .all(|(wire, expected)| wire.matches(expected))
        || prepared.canonical_bytes()? != bytes
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(prepared)
}

fn check_wire_bounds(instance: &ImportWire) -> Result<(), DeploymentError> {
    // Typed decoding rejects malformed values; canonical comparison below
    // rederives every immutable identity and binding instead of trusting them.
    if instance.volumes.len() > volume_domain::MAX_VOLUME_SLOTS
        || instance.parameters.len() > recipe_domain::MAX_RECIPE_ITEMS
        || instance.operation_id.is_nil()
        || instance.instance_id.as_uuid().is_nil()
        || instance.revision_id.as_uuid().is_nil()
        || instance.project_id.as_uuid().is_nil()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    for volume in &instance.volumes {
        if volume.volume_id.as_uuid().is_nil() || volume.grant_id.as_uuid().is_nil() {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    Ok(())
}

impl ImportWire {
    fn matches(&self, expected: &super::PreparedInstanceImport) -> bool {
        self.resource == *expected.resource()
            && self.operation_id == expected.operation_id()
            && self.command_key == expected.command_key()
            && self.instance_id == expected.instance_id()
            && self.revision_id == expected.revision_id()
            && self.project_id == expected.project_id()
            && self.name == *expected.name()
            && self.parameters == *expected.parameters()
            && self.volumes.len() == expected.volumes().len()
            && self
                .volumes
                .iter()
                .zip(expected.volumes())
                .all(|(wire, actual)| {
                    wire.slot == *actual.slot()
                        && wire.guest_path == *actual.guest_path()
                        && wire.access_mode == actual.access_mode()
                        && wire.volume_id == actual.volume_id()
                        && wire.grant_id == actual.grant_id()
                })
    }
}

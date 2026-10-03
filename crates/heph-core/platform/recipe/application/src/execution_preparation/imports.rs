use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use recipe_domain::{ReleasePin, ResolvedResource};
use release_domain::{
    InstanceName, ParameterName, ParameterValue, ReleaseCommandKey, RuntimePolicy,
};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, VolumeId};
use serde::Serialize;
use uuid::Uuid;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeMountGrantId};

use crate::{
    CommandIdentity, DeploymentError, DeploymentIntent, PlannedResource, PlannedResourceIdentity,
    SourceObservation,
};

use super::mapping;

/// Exact future import selection; its predicted grant is not an authority grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedVolumeSelection {
    slot: CapabilitySlotKey,
    guest_path: GuestMountPath,
    access_mode: VolumeAccessMode,
    volume_id: VolumeId,
    grant_id: VolumeMountGrantId,
}

impl PreparedVolumeSelection {
    /// Returns the released symbolic slot.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }
    /// Returns the exact released guest path.
    #[must_use]
    pub const fn guest_path(&self) -> &GuestMountPath {
        &self.guest_path
    }
    /// Returns the released access mode.
    #[must_use]
    pub const fn access_mode(&self) -> VolumeAccessMode {
        self.access_mode
    }
    /// Returns the predicted owned or explicit external volume.
    #[must_use]
    pub const fn volume_id(&self) -> VolumeId {
        self.volume_id
    }
    /// Returns the stable identity for a grant that import must actually authorize.
    #[must_use]
    pub const fn grant_id(&self) -> VolumeMountGrantId {
        self.grant_id
    }
}

/// All typed import fields and exact source observations, without provider handles.
///
/// Released pin and policies are stored in `source`; the accessors project the
/// exact fields required by `ImportAgentWithVolumes` without recomputing config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedInstanceImport {
    resource: CapabilitySlotKey,
    operation_id: Uuid,
    command_key: ReleaseCommandKey,
    instance_id: AgentInstanceId,
    revision_id: AgentInstanceRevisionId,
    project_id: ProjectId,
    name: InstanceName,
    parameters: BTreeMap<ParameterName, ParameterValue>,
    source: SourceObservation,
    volumes: Vec<PreparedVolumeSelection>,
}

impl PreparedInstanceImport {
    /// Returns the graph resource name.
    #[must_use]
    pub const fn resource(&self) -> &CapabilitySlotKey {
        &self.resource
    }
    /// Returns the tagged backend operation identity, independent of attempts.
    #[must_use]
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }
    /// Returns the actual release-service command key, which is a SHA-256 value.
    #[must_use]
    pub const fn command_key(&self) -> ReleaseCommandKey {
        self.command_key
    }
    /// Returns the unchanged predicted consumer identity.
    #[must_use]
    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance_id
    }
    /// Returns the unchanged predicted initial revision.
    #[must_use]
    pub const fn revision_id(&self) -> AgentInstanceRevisionId {
        self.revision_id
    }
    /// Returns the owning project.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }
    /// Returns the persisted project-scoped name.
    #[must_use]
    pub const fn name(&self) -> &InstanceName {
        &self.name
    }
    /// Returns resolved/defaulted ordinary values without secrets.
    #[must_use]
    pub const fn parameters(&self) -> &BTreeMap<ParameterName, ParameterValue> {
        &self.parameters
    }
    /// Returns checked original source facts; these are not grants.
    #[must_use]
    pub const fn source(&self) -> &SourceObservation {
        &self.source
    }
    /// Returns the exact release/export pin for import.
    #[must_use]
    pub const fn release_pin(&self) -> ReleasePin {
        self.source.pin()
    }
    /// Returns the exact released policy selection.
    #[must_use]
    pub const fn selected_policy(&self) -> &RuntimePolicy {
        self.source.selected_policy()
    }
    /// Returns the frozen configured platform ceiling.
    #[must_use]
    pub const fn platform_policy(&self) -> &RuntimePolicy {
        self.source.platform().policy()
    }
    /// Returns the frozen platform policy version.
    #[must_use]
    pub fn platform_policy_version(&self) -> &str {
        self.source.platform().version()
    }
    /// Returns complete required/selected optional slots in canonical slot order.
    #[must_use]
    pub fn volumes(&self) -> &[PreparedVolumeSelection] {
        &self.volumes
    }
    /// Fingerprints the complete import record, distinct from release actor hashing.
    ///
    /// # Errors
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn input_hash(&self) -> Result<release_domain::ContentHash, DeploymentError> {
        serde_json::to_vec(self)
            .map(|bytes| release_domain::ContentHash::digest(&bytes))
            .map_err(|_| DeploymentError::Serialization)
    }
}

pub fn prepare(
    intent: &DeploymentIntent,
    command: CommandIdentity,
    name: &CapabilitySlotKey,
    resource: &PlannedResource,
    source: &SourceObservation,
) -> Result<PreparedInstanceImport, DeploymentError> {
    let (
        PlannedResourceIdentity::Instance { id, revision_id },
        ResolvedResource::Instance(instance),
    ) = (resource.identity(), resource.intent())
    else {
        return Err(DeploymentError::IntentMismatch);
    };
    let mut volumes = instance
        .volume_bindings
        .iter()
        .map(|binding| {
            let selected = intent
                .resources()
                .get(&binding.resource)
                .ok_or(DeploymentError::IntentMismatch)?;
            let PlannedResourceIdentity::Volume { id, .. } = selected.identity() else {
                return Err(DeploymentError::IntentMismatch);
            };
            Ok(PreparedVolumeSelection {
                slot: binding.slot.clone(),
                guest_path: binding.guest_path.clone(),
                access_mode: binding.access_mode,
                volume_id: id,
                grant_id: VolumeMountGrantId::from_uuid(mapping::mount_grant(
                    revision_id.as_uuid(),
                    &binding.slot,
                ))
                .map_err(|_| DeploymentError::InvalidIdentifier)?,
            })
        })
        .collect::<Result<Vec<_>, DeploymentError>>()?;
    volumes.sort_by(|left, right| left.slot.cmp(&right.slot));
    let operation_id = mapping::import_operation(intent.id(), name);
    Ok(PreparedInstanceImport {
        resource: name.clone(),
        operation_id,
        command_key: mapping::import_key(intent.id(), command, name, operation_id),
        instance_id: id,
        revision_id,
        project_id: intent.project_id(),
        name: InstanceName::parse(format!("recipe-{}", id.as_uuid()))
            .map_err(|_| DeploymentError::InvalidPlanningInput)?,
        parameters: instance.parameters.clone(),
        source: source.clone(),
        volumes,
    })
}

//! Exact initial imports and permanent removal admission for volume consumers.

use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use release_domain::{
    AgentInstanceId, AgentInstanceRevisionId, InstanceName, InstanceRemovalId, InstanceVolumeMode,
    ParameterName, ParameterValue, ReleaseAgentId, ReleaseCommandKey, ReleaseId, RuntimePolicy,
};
use runtime_types::VolumeId;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeMountGrantId};

/// One exact resource selection and predicted immutable mount grant.
///
/// Storage compares the path and mode to the published declaration. Selection
/// alone establishes neither attachment authority nor runtime support.
#[derive(Debug, Clone)]
pub struct VolumeSlotSelection {
    /// Exact declared symbolic slot.
    pub slot: CapabilitySlotKey,
    /// Exact declared guest mount path.
    pub guest_path: GuestMountPath,
    /// Exact declared access mode.
    pub access_mode: VolumeAccessMode,
    /// Stable project-owned resource.
    pub volume_id: VolumeId,
    /// Predicted ID for this deliberate authority grant.
    pub grant_id: VolumeMountGrantId,
}

/// Atomically imports one exact published export with complete volume bindings.
///
/// Parameters resolve against the frozen schema. Selections are normalized by
/// slot before hashing. The initial revision can be runnable while dispatch is
/// separately closed until the named-volume runtime profile is supported.
#[derive(Debug, Clone)]
pub struct ImportAgentWithVolumes {
    /// Stable command identity; retries must preserve the complete input.
    pub command_key: ReleaseCommandKey,
    /// Predicted consumer identity.
    pub instance_id: AgentInstanceId,
    /// Predicted initial immutable revision.
    pub revision_id: AgentInstanceRevisionId,
    /// Exact published release pin.
    pub release_id: ReleaseId,
    /// Exact exported agent within that release.
    pub release_agent_id: ReleaseAgentId,
    /// Target owning project.
    pub project_id: ProjectId,
    /// Project-scoped instance name.
    pub name: InstanceName,
    /// Ordinary parameters, resolved without secrets.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Consumer policy selection within the release ceiling.
    pub selected_policy: RuntimePolicy,
    /// Current platform ceiling.
    pub platform_policy: RuntimePolicy,
    /// Exact version of that platform ceiling.
    pub platform_policy_version: String,
    /// Complete required and selected optional slots.
    pub volumes: Vec<VolumeSlotSelection>,
}

/// Current-manager admission to permanently stop future instance launches.
#[derive(Debug, Clone)]
pub struct RequestInstanceRemoval {
    /// Stable idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Predicted permanent closure identity.
    pub removal_id: InstanceRemovalId,
    /// Exact instance being closed.
    pub instance_id: AgentInstanceId,
    /// Compare-and-swap version observed before admission.
    pub expected_version: u64,
}

/// Retained admission evidence, without a claim of physical cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceRemovalAdmission {
    /// Immutable permanent closure identity.
    pub removal_id: InstanceRemovalId,
    /// Closed consumer instance.
    pub instance_id: AgentInstanceId,
}

/// Safe live attachment-profile projection; contains no provider handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceVolumeStatus {
    /// Exact consumer identity.
    pub instance_id: AgentInstanceId,
    /// Current compare-and-swap version for removal admission.
    pub version: u64,
    /// Immutable profile chosen at creation.
    pub mode: InstanceVolumeMode,
    /// Whether the installed runtime supports this profile.
    pub dispatch_supported: bool,
    /// Current independent launch gate.
    pub run_gate_open: bool,
    /// Permanent closure admission, without terminal cleanup proof.
    pub removal_id: Option<InstanceRemovalId>,
}

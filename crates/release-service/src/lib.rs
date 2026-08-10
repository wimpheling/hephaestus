//! Provider-neutral release command DTOs and workflow ports.

use capability_domain::{
    CapabilityBindingId, CapabilityOperation, CapabilityResource, CapabilitySlotKey,
};
use forge_domain::{ProjectId, RepositoryId};
use git_capability_domain::GitCapabilityCeiling;
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, ArtifactKind,
    ArtifactPath, BuildRequestId, ContentHash, InstanceName, ParameterName, ParameterValue,
    RefSelector, ReleaseAgentId, ReleaseArtifactId, ReleaseCommandKey, ReleaseId, ReleaseVersion,
    RuntimePolicy, TriggerPolicy,
};
use runtime_types::RunId;
use std::collections::BTreeMap;
use uuid::Uuid;

/// One already safely imported immutable artifact.
#[derive(Debug, Clone)]
pub struct ReleaseArtifactInput {
    /// Stable artifact identity.
    pub id: ReleaseArtifactId,
    /// Normalized path.
    pub path: ArtifactPath,
    /// Artifact kind.
    pub kind: ArtifactKind,
    /// Unix permission bits.
    pub mode: u16,
    /// Exact SHA-256 content hash.
    pub content_hash: ContentHash,
    /// Byte length.
    pub size_bytes: u64,
    /// Bounded media type.
    pub media_type: String,
    /// Opaque canonical storage key.
    pub storage_key: Uuid,
}

/// Trusted worker command that turns a complete safely imported build into a
/// draft release.
#[derive(Debug, Clone)]
pub struct CompleteBuild {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact completed build.
    pub build_request_id: BuildRequestId,
    /// Stable draft release.
    pub release_id: ReleaseId,
    /// Repository-scoped release version.
    pub version: ReleaseVersion,
    /// Stable release export identity.
    pub release_agent_id: ReleaseAgentId,
    /// Complete immutable artifacts.
    pub artifacts: Vec<ReleaseArtifactInput>,
}

/// Project import command.
#[derive(Debug, Clone)]
pub struct ImportAgent {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Stable product-level instance.
    pub instance_id: AgentInstanceId,
    /// Stable initial revision.
    pub revision_id: AgentInstanceRevisionId,
    /// Consuming project.
    pub project_id: ProjectId,
    /// Published reusable export.
    pub release_agent_id: ReleaseAgentId,
    /// Project-scoped name.
    pub name: InstanceName,
    /// Explicit typed values.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Project resource/network restriction.
    pub selected_policy: RuntimePolicy,
    /// Current platform ceiling.
    pub platform_policy: RuntimePolicy,
    /// Current platform policy version.
    pub platform_policy_version: String,
}

/// Repository/ref attachment command.
#[derive(Debug, Clone)]
pub struct CreateAttachment {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Stable attachment.
    pub attachment_id: AgentAttachmentId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Target repository.
    pub repository_id: RepositoryId,
    /// Exact or prefix ref selection.
    pub ref_selector: RefSelector,
    /// Trigger behavior.
    pub trigger_policy: TriggerPolicy,
}

/// Enables or disables one exact attachment without changing provenance.
#[derive(Debug, Clone)]
pub struct SetAttachmentEnabled {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact attachment.
    pub attachment_id: AgentAttachmentId,
    /// Desired trigger state.
    pub enabled: bool,
}

/// Tombstones one exact attachment.
#[derive(Debug, Clone)]
pub struct RemoveAttachment {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact attachment.
    pub attachment_id: AgentAttachmentId,
}

/// Creates and activates a new immutable parameter/resource revision.
#[derive(Debug, Clone)]
pub struct ReviseInstance {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Parent project instance.
    pub instance_id: AgentInstanceId,
    /// Compare-and-swap expected active revision.
    pub expected_revision_id: AgentInstanceRevisionId,
    /// Stable new immutable revision.
    pub new_revision_id: AgentInstanceRevisionId,
    /// Complete explicit typed parameter overrides.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// New project restriction within release bounds.
    pub selected_policy: RuntimePolicy,
    /// Current platform ceiling.
    pub platform_policy: RuntimePolicy,
    /// Current platform policy version.
    pub platform_policy_version: String,
}

/// One explicit exact resource selection for a released capability slot.
#[derive(Debug, Clone)]
pub struct CapabilityBindingSelection {
    /// Stable binding identity retained by historical revisions and runs.
    pub binding_id: CapabilityBindingId,
    /// Symbolic release slot being satisfied.
    pub slot: CapabilitySlotKey,
    /// Exact selected Hephaestus resource.
    pub resource: CapabilityResource,
    /// Explicit operation ceiling granted to the workload.
    pub granted_operations: Vec<CapabilityOperation>,
    /// Optional exact Git authority rules selected below the release ceiling.
    /// `None` selects the exact release ceiling for a typed Git slot. A value
    /// may only narrow that ceiling. This remains absent for non-Git slots.
    pub git_authority: Option<GitCapabilityCeiling>,
}

/// Creates a new immutable instance revision with a complete capability set.
#[derive(Debug, Clone)]
pub struct ReviseInstanceCapabilities {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Parent project-owned instance.
    pub instance_id: AgentInstanceId,
    /// Compare-and-swap expected active revision.
    pub expected_revision_id: AgentInstanceRevisionId,
    /// Stable new immutable revision.
    pub new_revision_id: AgentInstanceRevisionId,
    /// Complete desired binding set; omitted optional slots remain unbound.
    pub bindings: Vec<CapabilityBindingSelection>,
    /// Canonical authorization model used to evaluate each grant.
    pub authorization_model_version: String,
}

/// Durable result of a capability revision operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRevisionResult {
    /// Newly activated immutable revision.
    pub revision_id: AgentInstanceRevisionId,
    /// Whether all revision requirements are currently satisfied.
    pub runnable: bool,
    /// Stable non-sensitive diagnostics for unmet requirements.
    pub diagnostics: Vec<CapabilityRevisionDiagnostic>,
}

/// One stable non-sensitive capability revision diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRevisionDiagnostic {
    /// Machine-readable diagnostic code.
    pub code: String,
    /// Symbolic release capability slot.
    pub slot: CapabilitySlotKey,
}

/// Creates a fully resolved candidate release update and closes the normal run
/// gate only when the candidate is runnable and state-compatible.
#[derive(Debug, Clone)]
pub struct CreateInstanceUpdate {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Stable update identity delivered to the hook.
    pub update_id: AgentUpdateId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Compare-and-swap current revision.
    pub expected_revision_id: AgentInstanceRevisionId,
    /// Stable candidate revision.
    pub candidate_revision_id: AgentInstanceRevisionId,
    /// Published candidate export in the exact same family.
    pub candidate_release_agent_id: ReleaseAgentId,
    /// Candidate typed parameters.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Candidate project restriction.
    pub selected_policy: RuntimePolicy,
    /// Current platform ceiling.
    pub platform_policy: RuntimePolicy,
    /// Current platform policy version.
    pub platform_policy_version: String,
}

/// Agent update-hook terminal result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateHookResult {
    /// Exit zero: agent committed its state migration.
    Committed,
    /// Explicit nonzero exit: agent reports it rolled its own state back.
    Rejected(i32),
    /// Signal, timeout, VM loss, or protocol uncertainty.
    Uncertain,
}

/// Durable platform decision after a hook terminal result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateDecision {
    /// Candidate revision became active and the gate reopened.
    Activated,
    /// Current revision remains active and the gate reopened.
    AgentRejected,
    /// Current revision remains selected but the instance is paused.
    CompatibilityUnknown,
    /// Hook committed but activation needs operator recovery.
    ActivationRecovery,
}

/// Starts the isolated update hook after the pre-gate run set drains.
#[derive(Debug, Clone)]
pub struct BeginUpdateHook {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact update.
    pub update_id: AgentUpdateId,
    /// Stable special update run created atomically with hook admission.
    pub hook_run_id: RunId,
}

/// Explicit operator choice for an update paused at a recovery boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRecoveryAction {
    /// Re-run an idempotent hook with the same stable update identity.
    RetryHook,
    /// Keep the current revision and accept responsibility for agent-owned state.
    RejectCandidate,
    /// Finish activation after the hook's durable success commit point.
    ResumeActivation,
}

impl UpdateRecoveryAction {
    /// Stable operation name persisted in command idempotency records.
    #[must_use]
    pub const fn operation(self) -> &'static str {
        match self {
            Self::RetryHook => "recover_update_retry",
            Self::RejectCandidate => "recover_update_reject",
            Self::ResumeActivation => "recover_update_resume",
        }
    }
}

/// Authorized, idempotent recovery command for one paused update.
#[derive(Debug, Clone)]
pub struct RecoverInstanceUpdate {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact update retaining its stable guest-visible identity.
    pub update_id: AgentUpdateId,
    /// Explicit recovery choice.
    pub action: UpdateRecoveryAction,
}

/// Durable result of an explicit update recovery command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRecoveryDecision {
    /// The same update may start a new hook attempt after normal-work drain.
    HookRetryScheduled,
    /// The prior revision remains selected and its run gate is open.
    CandidateRejected,
    /// The hook-committed candidate is active and its run gate is open.
    CandidateActivated,
}

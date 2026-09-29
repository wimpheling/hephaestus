use forge_domain::{CommitSha, GitRef, ProjectId, RepositoryId};
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId, RunId, VolumeId,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    AgentAttachmentId, AgentFamilyId, AgentUpdateId, ContentHash, DeferredTriggerId, InstanceName,
    InstanceState, ParameterDocument, RefSelector, RuntimePolicy, UpdateState,
};

/// Project-owned instance aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstance {
    /// Stable product-level identity.
    pub id: AgentInstanceId,
    /// Consuming project.
    pub project_id: ProjectId,
    /// Project-scoped name.
    pub name: InstanceName,
    /// Compatible source family.
    pub family_id: AgentFamilyId,
    /// Current lifecycle.
    pub state: InstanceState,
    /// Exact active immutable revision.
    pub active_revision_id: AgentInstanceRevisionId,
    /// Optional one-per-instance state volume.
    pub state_volume_id: Option<VolumeId>,
    /// Optimistic aggregate version.
    pub version: u64,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Immutable completely resolved instance revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstanceRevision {
    /// Revision identifier.
    pub id: AgentInstanceRevisionId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Exact immutable release export.
    pub release_agent_id: ReleaseAgentId,
    /// Validated parameter document.
    pub parameters: ParameterDocument,
    /// Opaque secret binding UUIDs. Plaintext and ciphertext cannot appear.
    pub secret_binding_ids: Vec<Uuid>,
    /// Fully resolved effective runtime policy.
    pub effective_policy: RuntimePolicy,
    /// Platform policy version used for validation.
    pub platform_policy_version: String,
    /// Whether this revision can currently run.
    pub runnable: bool,
    /// Stable non-sensitive invalidity diagnostics.
    pub diagnostics: Vec<RevisionDiagnostic>,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Stable revision validation diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionDiagnostic {
    /// Stable code.
    pub code: String,
    /// Non-sensitive field key.
    pub field: Option<String>,
}

/// Attachment trigger policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerPolicy {
    /// Accepted pushes matching the ref selector create requests.
    Push,
    /// Only explicit authorized commands create requests.
    Manual,
    /// Both push and explicit commands are accepted.
    PushAndManual,
}

/// One repository/ref attachment for a reusable instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAttachment {
    /// Attachment identity.
    pub id: AgentAttachmentId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Target repository in the consuming project.
    pub repository_id: RepositoryId,
    /// Bounded ref selection.
    pub ref_selector: RefSelector,
    /// Trigger policy owned by this attachment.
    pub trigger_policy: TriggerPolicy,
    /// Whether new triggers are accepted.
    pub enabled: bool,
    /// Tombstone time. Historical runs keep resolving this row.
    pub removed_at: Option<OffsetDateTime>,
}

/// Exact candidate update with compare-and-swap provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUpdate {
    /// Stable update identity delivered to hooks.
    pub id: AgentUpdateId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Revision expected to remain active until the update commits.
    pub expected_current_revision_id: AgentInstanceRevisionId,
    /// Fully validated candidate.
    pub candidate_revision_id: AgentInstanceRevisionId,
    /// Durable lifecycle including irreversible hook success.
    pub state: UpdateState,
    /// Optional exact update-hook run.
    pub hook_run_id: Option<RunId>,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Trigger accepted while an instance run gate is closed. It deliberately has
/// no instance revision field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredTrigger {
    /// Stable trigger identity.
    pub id: DeferredTriggerId,
    /// Matching attachment.
    pub attachment_id: AgentAttachmentId,
    /// Exact target repository.
    pub repository_id: RepositoryId,
    /// Exact target ref.
    pub target_ref: GitRef,
    /// Exact target commit.
    pub target_commit: CommitSha,
    /// Source receive or command UUID.
    pub source_id: Uuid,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Protected exact provenance for one normal run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactRunProvenance {
    /// Run identity.
    pub run_id: RunId,
    /// Product-level agent.
    pub instance_id: AgentInstanceId,
    /// Exact immutable revision.
    pub instance_revision_id: AgentInstanceRevisionId,
    /// Exact source release.
    pub release_id: ReleaseId,
    /// Exact release export.
    pub release_agent_id: ReleaseAgentId,
    /// Exact attachment.
    pub attachment_id: AgentAttachmentId,
    /// Exact target repository.
    pub target_repository_id: RepositoryId,
    /// Exact target ref.
    pub target_ref: GitRef,
    /// Exact target commit.
    pub target_commit: CommitSha,
    /// Immutable parameter hash.
    pub parameter_hash: ContentHash,
    /// Platform policy version verified at dispatch.
    pub platform_policy_version: String,
}

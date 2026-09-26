//! Provider-neutral secret command and dispatch DTOs.

use forge_domain::{CommitSha, GitRef};
use release_domain::{AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId};
use runtime_types::RunId;
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretLeaseId, SecretName, SecretOwner,
    SecretRuntimeSessionId, SecretSlotKey, SecretTarget, SecretUsePolicy, SecretValue,
    SecretVersionId,
};
use time::OffsetDateTime;
use uuid::Uuid;

/// Initial immutable secret version command. Plaintext has no formatting or
/// serialization implementation that can expose its contents.
pub struct CreateSecret {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable caller-selected secret identity.
    pub secret_id: SecretId,
    /// Stable first version identity.
    pub version_id: SecretVersionId,
    /// Exact owner.
    pub owner: SecretOwner,
    /// Owner-scoped name.
    pub name: SecretName,
    /// Owner ceiling.
    pub allowed_delivery_modes: Vec<DeliveryMode>,
    /// Initial plaintext accepted only at this boundary.
    pub value: SecretValue,
}

/// Rotate to one exact new immutable version.
pub struct RotateSecret {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Secret being rotated.
    pub secret_id: SecretId,
    /// Compare-and-swap expected active version.
    pub expected_active_version_id: SecretVersionId,
    /// Stable new version identity.
    pub new_version_id: SecretVersionId,
    /// Replacement plaintext.
    pub value: SecretValue,
}

/// Offer a bounded source grant to one exact target.
#[derive(Debug, Clone)]
pub struct GrantSecret {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable grant identity.
    pub grant_id: SecretGrantId,
    /// Source secret.
    pub secret_id: SecretId,
    /// Exact target.
    pub target: SecretTarget,
    /// Bounded delivery policy.
    pub policy: SecretUsePolicy,
    /// Optional expiration.
    pub expires_at: Option<OffsetDateTime>,
}

/// Accept one exact active source grant under a local alias.
#[derive(Debug, Clone)]
pub struct AcceptSecretImport {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable import identity.
    pub import_id: SecretImportId,
    /// Source grant.
    pub grant_id: SecretGrantId,
    /// Exact accepting target.
    pub target: SecretTarget,
    /// Target-local alias.
    pub alias: SecretAlias,
}

/// Atomically creates a source grant and accepts its target-side import.
///
/// This convenience command is intentionally available only when one actor
/// independently holds both source grant-management and target import-
/// acceptance authority.
#[derive(Debug, Clone)]
pub struct GrantAndAcceptSecretImport {
    /// Deterministic identity for the compound command.
    pub command_key: SecretCommandKey,
    /// Stable source grant identity.
    pub grant_id: SecretGrantId,
    /// Source secret.
    pub secret_id: SecretId,
    /// Exact grant and import target.
    pub target: SecretTarget,
    /// Bounded use policy.
    pub policy: SecretUsePolicy,
    /// Optional grant expiration.
    pub expires_at: Option<OffsetDateTime>,
    /// Stable accepted import identity.
    pub import_id: SecretImportId,
    /// Target-local opaque alias.
    pub alias: SecretAlias,
}

/// Creates a new immutable instance revision whose declared symbolic slot is
/// bound to one eligible opaque import.
#[derive(Debug, Clone)]
pub struct BindSecret {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable new binding identity for the selected slot.
    pub binding_id: AgentSecretBindingId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Compare-and-swap active source revision.
    pub expected_revision_id: AgentInstanceRevisionId,
    /// Stable new immutable revision.
    pub new_revision_id: AgentInstanceRevisionId,
    /// Opaque target import.
    pub import_id: SecretImportId,
    /// Declared release slot.
    pub slot: SecretSlotKey,
    /// Exact delivery mode.
    pub mode: DeliveryMode,
    /// Selected declared phases.
    pub phases: Vec<ExecutionPhase>,
    /// Exact selected instance attachments.
    pub attachment_ids: Vec<Uuid>,
    /// Selected broker destination ceiling.
    pub destinations: Vec<String>,
}

/// Declares the immutable outbound HTTPS substitution rule for one already
/// bound brokered slot. This contains authority metadata only, never a secret
/// value or guest credential.
#[derive(Debug, Clone)]
pub struct DeclareBrokeredHttpsRule {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable immutable rule identity.
    pub rule_id: uuid::Uuid,
    /// Existing active brokered binding.
    pub binding_id: AgentSecretBindingId,
    /// Exact HTTPS origin.
    pub destination: String,
    /// Exact outbound header name.
    pub header: String,
    /// Optional fixed prefix; absent means complete-value substitution.
    pub header_prefix: Option<String>,
}

/// Resolves one exact runnable instance revision immediately before dispatch.
#[derive(Debug, Clone)]
pub struct ResolveRunSecrets {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Caller-selected runtime session identity.
    pub session_id: SecretRuntimeSessionId,
    /// Existing pre-start run receiving the authority.
    pub run_id: RunId,
    /// Exact project-owned instance.
    pub instance_id: AgentInstanceId,
    /// Exact active immutable revision.
    pub instance_revision_id: AgentInstanceRevisionId,
    /// Exact enabled target attachment for a normal run.
    pub attachment_id: Option<AgentAttachmentId>,
    /// Exact target ref for a normal run.
    pub target_ref: Option<GitRef>,
    /// Exact target commit for a normal run.
    pub target_commit: Option<CommitSha>,
    /// Normal run or update hook.
    pub phase: ExecutionPhase,
    /// Short runtime-authority expiry.
    pub expires_at: OffsetDateTime,
}

/// One non-sensitive exact lease included in a runtime session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedSecretLease {
    /// Stable lease identity.
    pub lease_id: SecretLeaseId,
    /// Symbolic release slot.
    pub slot: SecretSlotKey,
    /// Authorized delivery mode.
    pub mode: DeliveryMode,
    /// Exact immutable version pinned for this run.
    pub version_id: SecretVersionId,
}

/// Fresh bearer authority returned only by the successful dispatch call.
pub struct RuntimeSecretAuthority {
    /// Stable session identity persisted with its token hash.
    pub session_id: SecretRuntimeSessionId,
    /// Fresh opaque credential. Formatting and serialization are redacted.
    pub credential: secret_domain::OpaqueRuntimeCredential,
    /// Exact value-free lease metadata.
    pub leases: Vec<IssuedSecretLease>,
}

/// One authenticated raw value handed only to the ephemeral mount builder.
pub struct ResolvedRawSecret {
    /// Exact lease authorizing the materialization.
    pub lease_id: SecretLeaseId,
    /// Stable symbolic file name.
    pub slot: SecretSlotKey,
    /// Short-lived plaintext with redacted formatting and serialization.
    pub value: SecretValue,
}

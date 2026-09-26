use runtime_types::RunId;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    AgentSecretBindingId, AuthorityStatus, DeliveryMode, ExecutionPhase, GatewaySecretBindingId,
    GatewaySecretLeaseId, SecretAlias, SecretGrantId, SecretId, SecretImportId, SecretLeaseId,
    SecretName, SecretOwner, SecretSlotKey, SecretStatus, SecretTarget, SecretUsePolicy,
    SecretVersionId,
};

/// Durable metadata for an owned secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secret {
    /// Stable secret identifier.
    pub id: SecretId,
    /// Exact organization or project owner.
    pub owner: SecretOwner,
    /// Owner-scoped name.
    pub name: SecretName,
    /// Current lifecycle state.
    pub status: SecretStatus,
    /// Active immutable version.
    pub active_version_id: Option<SecretVersionId>,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Immutable encrypted-version metadata. Ciphertext is intentionally absent
/// from the provider-neutral domain model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretVersion {
    /// Version identifier.
    pub id: SecretVersionId,
    /// Parent secret.
    pub secret_id: SecretId,
    /// Monotonic owner-local version number.
    pub sequence: u64,
    /// Versioned encryption algorithm identifier.
    pub algorithm: String,
    /// Host-side key-encryption-key reference.
    pub key_reference: String,
    /// Plaintext length used for limits and audit.
    pub content_length: u32,
    /// Creation time.
    pub created_at: OffsetDateTime,
    /// Revocation time, if explicitly revoked.
    pub revoked_at: Option<OffsetDateTime>,
    /// Purge time, if cryptographic material was destroyed.
    pub purged_at: Option<OffsetDateTime>,
}

/// Explicit source-side authority offered to one exact target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretGrant {
    /// Grant identifier.
    pub id: SecretGrantId,
    /// Source secret.
    pub secret_id: SecretId,
    /// Exact target.
    pub target: SecretTarget,
    /// Bounded authority ceiling.
    pub policy: SecretUsePolicy,
    /// Current status.
    pub status: AuthorityStatus,
    /// Optional expiration.
    pub expires_at: Option<OffsetDateTime>,
}

/// Target-side acceptance of an opaque grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretImport {
    /// Import identifier.
    pub id: SecretImportId,
    /// Source grant. An import can never refer to another import.
    pub grant_id: SecretGrantId,
    /// Exact accepted target.
    pub target: SecretTarget,
    /// Target-local alias.
    pub alias: SecretAlias,
    /// Current status.
    pub status: AuthorityStatus,
}

/// Exact immutable agent-revision binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSecretBinding {
    /// Binding identifier.
    pub id: AgentSecretBindingId,
    /// Opaque accepted import.
    pub import_id: SecretImportId,
    /// Exact immutable agent-instance revision UUID.
    pub agent_instance_revision_id: Uuid,
    /// Symbolic release slot.
    pub slot: SecretSlotKey,
    /// Selected delivery authority.
    pub mode: DeliveryMode,
    /// Selected phases.
    pub phases: Vec<ExecutionPhase>,
    /// Exact selected attachments.
    pub attachment_ids: Vec<Uuid>,
    /// Normalized destination ceiling.
    pub destinations: Vec<String>,
    /// Current status.
    pub status: AuthorityStatus,
}

/// Short-lived authority bound to an exact run and secret version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretLease {
    /// Lease identifier.
    pub id: SecretLeaseId,
    /// Exact runtime run.
    pub run_id: RunId,
    /// Exact binding.
    pub binding_id: AgentSecretBindingId,
    /// Exact immutable version.
    pub secret_version_id: SecretVersionId,
    /// Authorized mode.
    pub mode: DeliveryMode,
    /// Credential hash; the bearer token is never stored.
    pub runtime_credential_hash: [u8; 32],
    /// Expiration.
    pub expires_at: OffsetDateTime,
    /// Current status.
    pub status: AuthorityStatus,
    /// Whether raw material may already have been observed by the guest.
    pub raw_material_observed: bool,
}

/// Short-lived secret authority for one exact gateway invocation.
///
/// The gateway's existing opaque runtime-session credential authenticates this
/// lease; this record deliberately adds no second credential and no plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySecretLease {
    /// Lease identifier.
    pub id: GatewaySecretLeaseId,
    /// Exact public gateway invocation UUID.
    pub invocation_id: Uuid,
    /// Exact immutable gateway-secret binding.
    pub binding_id: GatewaySecretBindingId,
    /// Exact immutable secret version selected for this invocation.
    pub secret_version_id: SecretVersionId,
    /// Exact immutable inbound substitution rule UUID.
    pub brokered_rule_id: Uuid,
    /// Expiration inherited from the gateway runtime authority session.
    pub expires_at: OffsetDateTime,
    /// Current authority status.
    pub status: AuthorityStatus,
}

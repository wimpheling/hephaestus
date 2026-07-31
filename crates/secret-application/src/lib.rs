//! Provider-neutral secret command, runtime, and broker contracts.

use async_trait::async_trait;
use forge_domain::{CommitSha, GitRef};
use identity_domain::AuthenticatedIdentity;
use release_domain::{AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId};
use runtime_types::RunId;
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretLeaseId, SecretName, SecretOwner,
    SecretRuntimeSessionId, SecretSlotKey, SecretTarget, SecretUsePolicy, SecretValue,
    SecretVersionId,
};
use secret_store::SecretStoreError;
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

/// Bounded semantic broker request from one exact runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerRequest {
    /// Claimed run identity, matched against the opaque credential.
    pub run_id: RunId,
    /// Symbolic released slot.
    pub slot: SecretSlotKey,
    /// Exact allowlisted destination name.
    pub destination: String,
    /// Adapter-defined semantic operation, never an arbitrary URL.
    pub operation: String,
    /// Bounded application body.
    pub body: Vec<u8>,
}

/// Sanitized broker response without upstream headers or credential material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerResponse {
    /// Provider-neutral result status.
    pub status: BrokerStatus,
    /// Bounded adapter-sanitized response body.
    pub body: Vec<u8>,
}

/// Provider-neutral broker outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerStatus {
    /// Semantic operation completed.
    Succeeded,
    /// Upstream rejected the semantic operation.
    Rejected,
    /// A retry may succeed without changing the request.
    Retryable,
}

/// Host-only application adapter. Implementations own DNS, transport,
/// redirect, metadata-endpoint, and response sanitization policy.
#[async_trait]
pub trait BrokerAdapter: Send + Sync {
    /// Applies the exact credential to one semantic allowlisted operation.
    ///
    /// Implementations must not return upstream authorization headers,
    /// credential-bearing redirects, or raw provider error bodies.
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError>;
}

/// Sanitized adapter failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BrokerAdapterError {
    /// The semantic operation is not allowed.
    #[error("broker operation is rejected")]
    Rejected,
    /// The provider may be retried.
    #[error("broker provider is temporarily unavailable")]
    Retryable,
}

/// Identifiers returned after creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatedSecret {
    /// Owned secret.
    pub secret_id: SecretId,
    /// Active immutable version.
    pub version_id: SecretVersionId,
}
/// Non-sensitive secret command failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretServiceError {
    /// Authorization denied the exact operation.
    #[error("secret command is not authorized")]
    AuthorizationDenied,
    /// Secret/grant/target is missing, hidden, revoked, or expired.
    #[error("secret authority is unavailable")]
    Unavailable,
    /// A target belongs to another organization.
    #[error("secret delegation cannot cross organization boundaries")]
    CrossOrganization,
    /// An accepted target differs from the source grant.
    #[error("secret grant target does not match the accepting target")]
    TargetMismatch,
    /// Delivery mode ceiling is empty or malformed.
    #[error("secret delivery modes are invalid")]
    InvalidDeliveryModes,
    /// Expected active version lost a compare-and-swap race.
    #[error("secret active version changed concurrently")]
    StaleActiveVersion,
    /// Version sequence cannot be represented.
    #[error("secret version sequence is exhausted")]
    VersionSequenceExhausted,
    /// Purge was attempted while usable runtime leases exist.
    #[error("secret encrypted material is retained by active leases")]
    ActiveLeases,
    /// Lifecycle does not permit this operation.
    #[error("secret lifecycle does not permit this operation")]
    InvalidLifecycle,
    /// One command key was reused for another operation.
    #[error("secret command idempotency identity conflicts")]
    IdempotencyConflict,
    /// Stored idempotency result is incomplete.
    #[error("secret command idempotency record is invalid")]
    CorruptIdempotencyRecord,
    /// Stored ownership violates the database invariant.
    #[error("stored secret owner is invalid")]
    InvalidStoredData,
    /// Release does not declare the selected symbolic slot.
    #[error("release does not declare the selected secret slot")]
    SlotNotDeclared,
    /// Requested mode, phase, or destination exceeds a declaration or grant.
    #[error("secret binding policy exceeds its declaration or grant")]
    BindingPolicyMismatch,
    /// Selected attachments or instance fall outside the import target.
    #[error("secret binding is outside the import target scope")]
    BindingOutOfScope,
    /// Active instance revision changed during configuration.
    #[error("agent instance revision changed concurrently")]
    StaleInstanceRevision,
    /// Runtime authority must be short-lived and expire in the future.
    #[error("secret runtime lease lifetime is invalid")]
    InvalidLeaseLifetime,
    /// Bearer authority was already issued and cannot be replayed.
    #[error("secret runtime credential was already issued")]
    CredentialAlreadyIssued,
    /// Presented runtime credential is unknown, expired, or run-mismatched.
    #[error("secret runtime authentication is denied")]
    RuntimeAuthenticationDenied,
    /// Broker request exceeds its exact semantic capability.
    #[error("secret broker request is denied")]
    BrokerRequestDenied,
    /// Sanitized broker response exceeded the configured bound.
    #[error("secret broker response is too large")]
    BrokerResponseTooLarge,
    /// Application adapter rejected or could not complete the request.
    #[error(transparent)]
    BrokerAdapter(#[from] BrokerAdapterError),
    /// JSON normalization failed.
    #[error("secret binding policy serialization failed")]
    Serialization(#[from] serde_json::Error),
    /// Authorization provider failure.
    #[error(transparent)]
    Authorization(#[from] authz_domain::AuthzError),
    /// Domain validation failure.
    #[error(transparent)]
    Domain(#[from] secret_domain::SecretValueError),
    /// Encryption/key-provider failure.
    #[error(transparent)]
    Encryption(#[from] SecretStoreError),
    /// Secret persistence provider failed.
    #[error("secret persistence failed")]
    Persistence,
}

/// Provider-neutral command service implemented by a persistence adapter.
#[allow(missing_docs)]
#[async_trait]
pub trait SecretCommandService: Send + Sync {
    async fn create(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateSecret,
    ) -> Result<CreatedSecret, SecretServiceError>;
    async fn rotate(
        &self,
        identity: &AuthenticatedIdentity,
        command: RotateSecret,
    ) -> Result<SecretVersionId, SecretServiceError>;
    async fn grant(
        &self,
        identity: &AuthenticatedIdentity,
        command: GrantSecret,
    ) -> Result<SecretGrantId, SecretServiceError>;
    async fn accept_import(
        &self,
        identity: &AuthenticatedIdentity,
        command: AcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError>;
    async fn grant_and_accept(
        &self,
        identity: &AuthenticatedIdentity,
        command: GrantAndAcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError>;
    async fn bind(
        &self,
        identity: &AuthenticatedIdentity,
        command: BindSecret,
    ) -> Result<AgentInstanceRevisionId, SecretServiceError>;
    async fn revoke(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError>;
    async fn set_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
        enabled: bool,
    ) -> Result<(), SecretServiceError>;
    async fn purge(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError>;
}

/// Provider-neutral exact dispatch resolver.
#[allow(missing_docs)]
#[async_trait]
pub trait SecretDispatchResolver: Send + Sync {
    async fn resolve_for_dispatch(
        &self,
        identity: &AuthenticatedIdentity,
        command: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError>;
}

/// Provider-neutral runtime lease resolver and broker authorizer.
#[allow(missing_docs)]
#[async_trait]
pub trait SecretRuntimeResolver: Send + Sync {
    async fn receive_raw(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        run_id: RunId,
        slot: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError>;
    async fn use_brokered(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        request: &BrokerRequest,
        adapter: &dyn BrokerAdapter,
    ) -> Result<BrokerResponse, SecretServiceError>;
}

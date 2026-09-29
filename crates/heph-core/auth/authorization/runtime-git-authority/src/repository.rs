use super::RuntimeGitAuthorityError;
use async_trait::async_trait;
use capability_domain::{AuthorizationSnapshotId, RuntimeCredentialGeneration, RuntimeSessionId};
use git_capability_domain::{GitCapabilityHash, GitCapabilityScope, GitOperation, RepositoryId};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

/// Safe immutable metadata for one issued runtime Git credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRuntimeGitCredential {
    /// Exact generic runtime session owning the credential.
    pub runtime_session_id: RuntimeSessionId,
    /// Immutable generic authorization snapshot.
    pub authorization_snapshot_id: AuthorizationSnapshotId,
    /// Exact persisted Git binding.
    pub binding_id: Uuid,
    /// Exact bound repository.
    pub repository_id: RepositoryId,
    /// Normalized persisted Git scope hash.
    pub scope_hash: GitCapabilityHash,
    /// Shared exact bootstrap issuance generation.
    pub generation: RuntimeCredentialGeneration,
    /// Exclusive expiry inherited from the generic runtime session.
    pub expires_at: OffsetDateTime,
}

/// Authority returned only after credential, session, scope, repository,
/// operation, expiry, and live revocation checks pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedRuntimeGitAuthority {
    /// Exact generic runtime session.
    pub runtime_session_id: RuntimeSessionId,
    /// Immutable generic authorization snapshot.
    pub authorization_snapshot_id: AuthorizationSnapshotId,
    /// Exact persisted Git binding.
    pub binding_id: Uuid,
    /// Revalidated normalized scope hash.
    pub scope_hash: GitCapabilityHash,
    /// Complete immutable Git scope reconstructed by the host.
    pub scope: Arc<GitCapabilityScope>,
    /// Exact old commit required by trigger-safe publication, when configured.
    pub expected_parent: Option<String>,
    /// Trusted evaluation instant.
    pub evaluated_at: OffsetDateTime,
}

/// Hash-only durable credential persistence and authentication boundary.
#[async_trait]
pub trait RuntimeGitCredentialRepository: Send + Sync {
    /// Finds safe metadata for a previously issued exact-session credential.
    ///
    /// # Errors
    ///
    /// Returns a redacted persistence failure.
    async fn find(
        &self,
        session_id: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeGitCredential>, RuntimeGitAuthorityError>;

    /// Persists a verifier only if the generic session and Git snapshot are
    /// already durable and mutually consistent.
    ///
    /// # Errors
    ///
    /// Returns a redacted persistence or immutable-binding failure.
    async fn create(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        credential_hash: super::RuntimeGitCredentialHash,
    ) -> Result<StoredRuntimeGitCredential, RuntimeGitAuthorityError>;

    /// Authenticates and resolves complete current authority for one exact
    /// repository operation.
    ///
    /// # Errors
    ///
    /// Returns a redacted denial or persistence failure.
    async fn authenticate(
        &self,
        credential_hash: super::RuntimeGitCredentialHash,
        repository_id: RepositoryId,
        operation: GitOperation,
        evaluated_at: OffsetDateTime,
    ) -> Result<AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError>;
}

/// Temporary encrypted host storage for a runtime Git bearer.
pub trait RuntimeGitHandoffStore: Send + Sync {
    /// Creates one exact-session, exact-generation envelope.
    ///
    /// # Errors
    ///
    /// Returns a redacted handoff failure or duplicate-envelope signal.
    fn create(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        expires_at: OffsetDateTime,
    ) -> Result<super::RuntimeGitCredential, RuntimeGitAuthorityError>;

    /// Opens an existing unexpired envelope for idempotent redelivery.
    ///
    /// # Errors
    ///
    /// Returns a redacted handoff failure when the envelope is unavailable.
    fn open(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        now: OffsetDateTime,
    ) -> Result<super::RuntimeGitCredential, RuntimeGitAuthorityError>;

    /// Destroys the temporary envelope after acknowledgement or revocation.
    ///
    /// # Errors
    ///
    /// Returns a redacted handoff failure.
    fn destroy(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeGitAuthorityError>;

    /// Purges elapsed envelopes after host or worker crashes.
    ///
    /// # Errors
    ///
    /// Returns a redacted handoff failure.
    fn purge_expired(&self, now: OffsetDateTime) -> Result<u64, RuntimeGitAuthorityError>;
}

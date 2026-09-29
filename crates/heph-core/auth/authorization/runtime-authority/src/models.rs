use super::RuntimeAuthorityError;
use async_trait::async_trait;
use capability_domain::{
    AuthorityHash, AuthorizationSnapshot, RuntimeCredential, RuntimeCredentialGeneration,
    RuntimeCredentialHash, RuntimeSessionId, RuntimeSessionIdentity, RuntimeSessionStatus,
};
use time::OffsetDateTime;
use uuid::Uuid;

/// Exact non-secret request to issue authority for one gateway invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayRuntimeSessionRequest {
    /// Session and invocation identity. Gateway sessions are one-to-one with
    /// their accepted invocation and never masquerade as agent runs.
    pub invocation_id: capability_domain::GatewayInvocationId,
    /// Durable gateway workload identity.
    pub gateway_id: Uuid,
    /// Exact immutable gateway revision selected at acceptance.
    pub gateway_revision_id: Uuid,
    /// Session issuance timestamp.
    pub issued_at: OffsetDateTime,
    /// Exclusive session expiry timestamp.
    pub expires_at: OffsetDateTime,
}

/// Trusted control-plane port for gateway invocation authority issuance.
#[async_trait]
pub trait GatewayRuntimeAuthorityIssuer: Send + Sync {
    /// Persists an immutable gateway snapshot and pending runtime session.
    async fn issue_gateway(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError>;

    /// Persists an immutable gateway snapshot and host-mediated service session.
    ///
    /// Host-mediated sessions are active without a guest acknowledgement and
    /// never create runtime bearer handoff material. Implementations that do
    /// not support service admission fail closed by default.
    async fn issue_gateway_service(
        &self,
        _request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        Err(RuntimeAuthorityError::Persistence)
    }
}

/// Immutable input persisted while issuing one runtime session.
pub struct NewRuntimeSession<'a> {
    /// Exact immutable authority ceiling.
    pub snapshot: &'a AuthorizationSnapshot,
    /// Identity bound to the snapshot and exact invocation.
    pub identity: &'a RuntimeSessionIdentity,
    /// Stable handoff generation. Retries must use the same value.
    pub generation: RuntimeCredentialGeneration,
    /// Hash-only verifier for bearer material retained by the host adapter.
    pub credential_hash: RuntimeCredentialHash,
    /// Optional exact repository attachment for an ordinary run.
    pub attachment_id: Option<Uuid>,
}

/// Safe persisted metadata for one runtime session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRuntimeSession {
    /// Session identifier.
    pub id: RuntimeSessionId,
    /// Exact authorization snapshot.
    pub snapshot_id: capability_domain::AuthorizationSnapshotId,
    /// Hash of the identity claims persisted with the session.
    pub identity_hash: AuthorityHash,
    /// Stable issuance generation.
    pub generation: RuntimeCredentialGeneration,
    /// Current lifecycle state.
    pub status: RuntimeSessionStatus,
    /// Stable issuance time used to reconstruct identity on redelivery.
    pub issued_at: OffsetDateTime,
    /// Exclusive expiry.
    pub expires_at: OffsetDateTime,
    /// Guest acknowledgement time, if acknowledged.
    pub acknowledged_at: Option<OffsetDateTime>,
    /// Permanent revocation time, if revoked.
    pub revoked_at: Option<OffsetDateTime>,
}

/// Durable hash-only runtime authority persistence.
#[async_trait]
pub trait RuntimeSessionRepository: Send + Sync {
    /// Finds safe metadata for an existing session.
    async fn find(
        &self,
        session_id: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError>;

    /// Atomically persists an immutable snapshot and pending session.
    async fn create(
        &self,
        session: NewRuntimeSession<'_>,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError>;

    /// Idempotently acknowledges the exact handoff generation.
    async fn acknowledge(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        acknowledged_at: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError>;

    /// Idempotently revokes a pending or active session.
    async fn revoke(
        &self,
        session_id: RuntimeSessionId,
        revoked_at: OffsetDateTime,
        reason: &str,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError>;

    /// Marks all elapsed pending or active sessions expired.
    async fn expire(&self, now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError>;
}

/// Trusted host storage for a temporary encrypted bootstrap handoff.
pub trait RuntimeHandoffStore: Send + Sync {
    /// Creates an envelope and returns the newly generated bearer material.
    ///
    /// Existing exact session/generation entries must fail closed; callers
    /// use [`Self::open`] for redelivery.
    ///
    /// # Errors
    ///
    /// Returns a safe handoff error if an exact envelope already exists or
    /// host encryption/storage is unavailable.
    fn create(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        expires_at: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError>;

    /// Opens an existing, unexpired exact envelope for bootstrap redelivery.
    ///
    /// # Errors
    ///
    /// Returns a safe handoff error if the envelope is missing, expired,
    /// corrupt, or cannot be decrypted by this host.
    fn open(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        now: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError>;

    /// Idempotently destroys one exact envelope.
    ///
    /// # Errors
    ///
    /// Returns a safe handoff error if host storage cannot remove the entry.
    fn destroy(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeAuthorityError>;

    /// Deletes elapsed envelopes after host or worker crashes.
    ///
    /// # Errors
    ///
    /// Returns a safe handoff error if the directory cannot be inspected or
    /// an elapsed entry cannot be removed.
    fn purge_expired(&self, now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError>;
}

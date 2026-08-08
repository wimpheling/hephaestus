//! Application contracts for issuing exact, short-lived runtime authority.
//!
//! `PostgreSQL` persists immutable snapshots and hash-only session records. A
//! separate trusted host adapter temporarily retains the encrypted bearer
//! credential until the guest acknowledges the exact issuance generation.

use async_trait::async_trait;
use capability_domain::{
    AuthorityHash, AuthorizationSnapshot, RuntimeCredential, RuntimeCredentialGeneration,
    RuntimeCredentialHash, RuntimeSessionId, RuntimeSessionIdentity, RuntimeSessionStatus,
};
use time::OffsetDateTime;
use uuid::Uuid;

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

/// Coordinates durable session issuance with temporary host-only handoff.
pub struct RuntimeSessionIssuer<R, H> {
    repository: R,
    handoff: H,
}

impl<R, H> RuntimeSessionIssuer<R, H>
where
    R: RuntimeSessionRepository,
    H: RuntimeHandoffStore,
{
    /// Creates an issuer from explicit persistence and host handoff adapters.
    #[must_use]
    pub const fn new(repository: R, handoff: H) -> Self {
        Self {
            repository,
            handoff,
        }
    }

    /// Issues or redelivers the same pending credential for an exact session.
    ///
    /// # Errors
    ///
    /// Fails closed on identity mismatch, an acknowledged/revoked/expired
    /// session, persistence failure, or unavailable/corrupt handoff material.
    pub async fn issue(
        &self,
        snapshot: &AuthorizationSnapshot,
        identity: &RuntimeSessionIdentity,
        attachment_id: Option<Uuid>,
        now: OffsetDateTime,
    ) -> Result<IssuedRuntimeSession, RuntimeAuthorityError> {
        let generation = RuntimeCredentialGeneration::INITIAL;
        if let Some(existing) = self.repository.find(identity.id()).await? {
            if existing.identity_hash != identity.normalized_hash()
                || existing.snapshot_id != snapshot.id()
            {
                return Err(RuntimeAuthorityError::IdentityMismatch);
            }
            if existing.status != RuntimeSessionStatus::PendingHandoff {
                return Err(RuntimeAuthorityError::SessionNotPending);
            }
            let credential = self.handoff.open(existing.id, existing.generation, now)?;
            return Ok(IssuedRuntimeSession {
                session: existing,
                credential,
            });
        }

        let credential = match self
            .handoff
            .create(identity.id(), generation, identity.expires_at())
        {
            Ok(credential) => credential,
            Err(RuntimeAuthorityError::HandoffExists) => {
                self.handoff.open(identity.id(), generation, now)?
            }
            Err(error) => return Err(error),
        };
        let credential_hash = credential.storage_hash(identity.id(), generation);
        let persisted = self
            .repository
            .create(NewRuntimeSession {
                snapshot,
                identity,
                generation,
                credential_hash,
                attachment_id,
            })
            .await;
        match persisted {
            Ok(session) => Ok(IssuedRuntimeSession {
                session,
                credential,
            }),
            Err(error) => match self.repository.find(identity.id()).await {
                Ok(Some(existing))
                    if existing.identity_hash == identity.normalized_hash()
                        && existing.snapshot_id == snapshot.id()
                        && existing.status == RuntimeSessionStatus::PendingHandoff =>
                {
                    Ok(IssuedRuntimeSession {
                        session: existing,
                        credential,
                    })
                }
                Ok(Some(_)) => Err(error),
                Ok(None) | Err(_) => {
                    let _ = self.handoff.destroy(identity.id(), generation);
                    Err(error)
                }
            },
        }
    }

    /// Records an exact-generation guest acknowledgement and destroys handoff.
    ///
    /// # Errors
    ///
    /// Returns a typed error for persistence or envelope deletion failure.
    pub async fn acknowledge(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        acknowledged_at: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let session = self
            .repository
            .acknowledge(session_id, generation, acknowledged_at)
            .await?;
        self.handoff.destroy(session_id, generation)?;
        Ok(session)
    }

    /// Permanently revokes authority and destroys any remaining handoff.
    ///
    /// # Errors
    ///
    /// Returns a typed error for persistence or envelope deletion failure.
    pub async fn revoke(
        &self,
        session_id: RuntimeSessionId,
        revoked_at: OffsetDateTime,
        reason: &str,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let session = self
            .repository
            .revoke(session_id, revoked_at, reason)
            .await?;
        self.handoff.destroy(session_id, session.generation)?;
        Ok(session)
    }

    /// Expires durable sessions and purges elapsed host handoff envelopes.
    ///
    /// # Errors
    ///
    /// Returns a typed persistence or host-handoff failure.
    pub async fn recover_expired(&self, now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        let expired = self.repository.expire(now).await?;
        let purged = self.handoff.purge_expired(now)?;
        Ok(expired.max(purged))
    }
}

/// One-time plaintext result delivered only to trusted bootstrap code.
pub struct IssuedRuntimeSession {
    /// Safe persisted metadata.
    pub session: StoredRuntimeSession,
    /// Opaque bearer material for the bootstrap channel.
    pub credential: RuntimeCredential,
}

/// Safe runtime authority issuance failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeAuthorityError {
    /// Durable storage was unavailable or rejected the operation.
    #[error("runtime authority persistence failed")]
    Persistence,
    /// The session does not exist or is deliberately hidden.
    #[error("runtime session is unavailable")]
    NotFound,
    /// Existing durable identity differs from the requested retry.
    #[error("runtime session identity does not match issuance request")]
    IdentityMismatch,
    /// A credential can no longer be delivered for this lifecycle state.
    #[error("runtime session is not pending handoff")]
    SessionNotPending,
    /// The acknowledgement generation differs from the issued generation.
    #[error("runtime session handoff generation does not match")]
    GenerationMismatch,
    /// The temporary host envelope is missing, expired, or cannot be opened.
    #[error("runtime credential handoff is unavailable")]
    HandoffUnavailable,
    /// An exact envelope already exists and must be redelivered instead.
    #[error("runtime credential handoff already exists")]
    HandoffExists,
}

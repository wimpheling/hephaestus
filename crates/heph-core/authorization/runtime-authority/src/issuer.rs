use super::{
    NewRuntimeSession, RuntimeAuthorityError, RuntimeHandoffStore, RuntimeSessionRepository,
    StoredRuntimeSession,
};
use capability_domain::{
    AuthorizationSnapshot, RuntimeCredential, RuntimeCredentialGeneration, RuntimeSessionId,
    RuntimeSessionIdentity, RuntimeSessionStatus,
};
use time::OffsetDateTime;
use uuid::Uuid;

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

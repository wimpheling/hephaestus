use super::{
    RuntimeGitAuthorityError, RuntimeGitCredential, RuntimeGitCredentialRepository,
    RuntimeGitHandoffStore, StoredRuntimeGitCredential,
};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use time::OffsetDateTime;

/// Coordinates durable verifier issuance with temporary host handoff.
pub struct RuntimeGitCredentialIssuer<R, H> {
    repository: R,
    handoff: H,
}

impl<R, H> RuntimeGitCredentialIssuer<R, H>
where
    R: RuntimeGitCredentialRepository,
    H: RuntimeGitHandoffStore,
{
    /// Creates an issuer from explicit persistence and handoff adapters.
    #[must_use]
    pub const fn new(repository: R, handoff: H) -> Self {
        Self {
            repository,
            handoff,
        }
    }

    /// Issues or redelivers the one non-renewable credential for a session.
    ///
    /// # Errors
    ///
    /// Fails closed when the generic session/scope are unavailable, a stored
    /// binding differs, or temporary bearer material cannot be recovered.
    pub async fn issue(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<IssuedRuntimeGitCredential, RuntimeGitAuthorityError> {
        if let Some(existing) = self.repository.find(session_id).await? {
            if existing.generation != generation || existing.expires_at != expires_at {
                return Err(RuntimeGitAuthorityError::IdentityMismatch);
            }
            let credential = self.handoff.open(session_id, generation, now)?;
            return Ok(IssuedRuntimeGitCredential {
                stored: existing,
                credential,
            });
        }

        let credential = match self.handoff.create(session_id, generation, expires_at) {
            Ok(credential) => credential,
            Err(RuntimeGitAuthorityError::HandoffExists) => {
                self.handoff.open(session_id, generation, now)?
            }
            Err(error) => return Err(error),
        };
        let persisted = self
            .repository
            .create(session_id, generation, credential.storage_hash())
            .await;
        match persisted {
            Ok(stored) => Ok(IssuedRuntimeGitCredential { stored, credential }),
            Err(error) => match self.repository.find(session_id).await? {
                Some(stored)
                    if stored.generation == generation && stored.expires_at == expires_at =>
                {
                    Ok(IssuedRuntimeGitCredential { stored, credential })
                }
                Some(_) => Err(error),
                None => {
                    let _ = self.handoff.destroy(session_id, generation);
                    Err(error)
                }
            },
        }
    }

    /// Removes bearer handoff after the shared bootstrap acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns a redacted handoff error.
    pub fn acknowledge_or_revoke(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeGitAuthorityError> {
        self.handoff.destroy(session_id, generation)
    }

    /// Purges expired host-only bearer envelopes.
    ///
    /// # Errors
    ///
    /// Returns a redacted handoff error.
    pub fn recover_expired(&self, now: OffsetDateTime) -> Result<u64, RuntimeGitAuthorityError> {
        self.handoff.purge_expired(now)
    }
}

/// Sensitive issue result delivered only through trusted bootstrap code.
pub struct IssuedRuntimeGitCredential {
    /// Safe durable metadata.
    pub stored: StoredRuntimeGitCredential,
    /// Opaque bearer material.
    pub credential: RuntimeGitCredential,
}

//! Application contracts for exact-run Git credential issuance.
//!
//! Runtime Git credentials are distinct from generic runtime credentials and
//! developer PATs. Only a verifier is durable; plaintext exists temporarily in
//! a host handoff envelope and the authenticated guest bootstrap stream.

use async_trait::async_trait;
use capability_domain::{AuthorizationSnapshotId, RuntimeCredentialGeneration, RuntimeSessionId};
use git_capability_domain::{GitCapabilityHash, GitCapabilityScope, GitOperation, RepositoryId};
use sha2::{Digest, Sha256};
use std::{fmt, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

/// Size of an opaque runtime Git credential.
pub const RUNTIME_GIT_CREDENTIAL_BYTES: usize = 32;

const CREDENTIAL_PREFIX: &str = "heph_git_v1_";
const HASH_DOMAIN: &[u8] = b"hephaestus.runtime-git-credential-verifier.v1\0";

/// One separately discriminated runtime Git bearer.
pub struct RuntimeGitCredential([u8; RUNTIME_GIT_CREDENTIAL_BYTES]);

impl RuntimeGitCredential {
    /// Creates a credential from cryptographically random secret bytes.
    #[must_use]
    pub const fn from_secret(secret: [u8; RUNTIME_GIT_CREDENTIAL_BYTES]) -> Self {
        Self(secret)
    }

    /// Parses the canonical password representation accepted by Git HTTP.
    ///
    /// # Errors
    ///
    /// Returns a redacted error for malformed or incorrectly sized input.
    pub fn parse(value: &str) -> Result<Self, RuntimeGitAuthorityError> {
        use base64::Engine as _;

        let encoded = value
            .strip_prefix(CREDENTIAL_PREFIX)
            .ok_or(RuntimeGitAuthorityError::InvalidCredential)?;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| RuntimeGitAuthorityError::InvalidCredential)?;
        let secret = decoded
            .try_into()
            .map_err(|_| RuntimeGitAuthorityError::InvalidCredential)?;
        Ok(Self(secret))
    }

    /// Returns the canonical password representation in zeroizing storage.
    #[must_use]
    pub fn expose_token(&self) -> Zeroizing<String> {
        use base64::Engine as _;

        Zeroizing::new(format!(
            "{CREDENTIAL_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.0)
        ))
    }

    /// Exposes raw bearer bytes only to authenticated bootstrap conversion.
    #[must_use]
    pub const fn expose(&self) -> &[u8; RUNTIME_GIT_CREDENTIAL_BYTES] {
        &self.0
    }

    /// Computes the domain-separated durable verifier.
    #[must_use]
    pub fn storage_hash(&self) -> RuntimeGitCredentialHash {
        let mut digest = Sha256::new();
        digest.update(HASH_DOMAIN);
        digest.update(self.0);
        RuntimeGitCredentialHash(digest.finalize().into())
    }
}

impl fmt::Debug for RuntimeGitCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeGitCredential([REDACTED])")
    }
}

impl Drop for RuntimeGitCredential {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Hash-only verifier persisted for one runtime Git bearer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeGitCredentialHash([u8; 32]);

impl RuntimeGitCredentialHash {
    /// Returns verifier bytes for persistence or indexed lookup.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

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
        credential_hash: RuntimeGitCredentialHash,
    ) -> Result<StoredRuntimeGitCredential, RuntimeGitAuthorityError>;

    /// Authenticates and resolves complete current authority for one exact
    /// repository operation.
    ///
    /// # Errors
    ///
    /// Returns a redacted denial or persistence failure.
    async fn authenticate(
        &self,
        credential_hash: RuntimeGitCredentialHash,
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
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError>;

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
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError>;

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

/// Redacted runtime Git credential failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeGitAuthorityError {
    /// Durable storage or scope reconstruction failed.
    #[error("runtime Git authority persistence failed")]
    Persistence,
    /// No exact live session/scope/credential matched.
    #[error("runtime Git authority is unavailable")]
    NotFound,
    /// Existing immutable issuance differs from this retry.
    #[error("runtime Git credential identity does not match")]
    IdentityMismatch,
    /// Presented password is not canonical runtime Git credential syntax.
    #[error("runtime Git credential is invalid")]
    InvalidCredential,
    /// Temporary bearer material is absent, expired, or corrupt.
    #[error("runtime Git credential handoff is unavailable")]
    HandoffUnavailable,
    /// An exact temporary envelope already exists.
    #[error("runtime Git credential handoff already exists")]
    HandoffExists,
}

#[cfg(test)]
mod tests {
    use super::RuntimeGitCredential;

    #[test]
    fn token_round_trip_is_canonical_redacted_and_domain_hashed() {
        let credential = RuntimeGitCredential::from_secret([0x5c; 32]);
        let token = credential.expose_token();
        assert!(token.starts_with("heph_git_v1_"));
        assert_eq!(
            RuntimeGitCredential::parse(&token)
                .expect("canonical token")
                .expose(),
            credential.expose()
        );
        assert!(!format!("{credential:?}").contains("92"));
        assert_ne!(credential.storage_hash().as_bytes(), *credential.expose());
        assert!(RuntimeGitCredential::parse("heph_pat_v1_not-runtime").is_err());
    }
}

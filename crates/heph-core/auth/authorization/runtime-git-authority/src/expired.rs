//! Closed expired comparison data, independent of Run/VM application crates.

use crate::{RuntimeGitAuthorityError, StoredRuntimeGitCredential};
use capability_domain::{
    AuthorityHash, AuthorizationSnapshot, AuthorizationSnapshotId, RuntimeCredentialGeneration,
    RuntimeCredentialHash, RuntimeInvocation, RuntimeSessionId, RuntimeSessionIdentity,
    RuntimeSessionStatus, WorkloadKind,
};
use std::fmt;
use time::OffsetDateTime;
use uuid::Uuid;

#[cfg(test)]
#[path = "expired/tests.rs"]
mod tests;

/// Unqualified input row metadata; only the closed ceiling checks coherence.
///
/// A repository must prove these values and the original graph independently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredRuntimeGitSessionMetadata {
    /// Exact generic session, not inferred from its Run UUID.
    pub session: RuntimeSessionId,
    /// Immutable generic authorization snapshot.
    pub snapshot: AuthorizationSnapshotId,
    /// Persisted digest of the complete identity claims.
    pub identity_hash: AuthorityHash,
    /// Original immutable issuance generation.
    pub generation: RuntimeCredentialGeneration,
    /// Actual observed lifecycle; must be Expired.
    pub status: RuntimeSessionStatus,
    /// Original issuance timestamp.
    pub issued_at: OffsetDateTime,
    /// Original exclusive expiry.
    pub expires_at: OffsetDateTime,
    /// Original acknowledgement, if one was recorded.
    pub acknowledged_at: Option<OffsetDateTime>,
    /// Observed revocation timestamp; Expired requires absence.
    pub revoked_at: Option<OffsetDateTime>,
}

/// Exact generic ceiling supporting a scoped Git handoff observation.
///
/// Construction proves internal equality only, not database provenance, current
/// authority or physical stop. Compare every generic ceiling field with the
/// generic issuer's observation before deleting either handoff. Observation
/// instants may differ; each must independently be at or after expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredRuntimeGitCeiling {
    run: Uuid,
    session: ExpiredRuntimeGitSessionMetadata,
    snapshot: AuthorizationSnapshot,
    identity: RuntimeSessionIdentity,
    generic_credential_hash: RuntimeCredentialHash,
    observed_at: OffsetDateTime,
}

impl ExpiredRuntimeGitCeiling {
    /// Checks a complete generic identity/snapshot against terminal row metadata.
    ///
    /// # Errors
    /// Rejects nil, non-expired, inconsistent or malformed lifecycle metadata.
    pub fn new(
        run: Uuid,
        session: ExpiredRuntimeGitSessionMetadata,
        snapshot: AuthorizationSnapshot,
        identity: RuntimeSessionIdentity,
        generic_credential_hash: RuntimeCredentialHash,
        observed_at: OffsetDateTime,
    ) -> Result<Self, RuntimeGitAuthorityError> {
        if run.is_nil()
            || session.session.as_uuid().is_nil()
            || session.snapshot.as_uuid().is_nil()
            || session.status != RuntimeSessionStatus::Expired
            || session.expires_at <= session.issued_at
            || observed_at < session.expires_at
            || session.revoked_at.is_some()
            || session
                .acknowledged_at
                .is_some_and(|at| at < session.issued_at || at >= session.expires_at)
            || session.session != identity.id()
            || session.snapshot != snapshot.id()
            || identity.snapshot_id() != snapshot.id()
            || identity.snapshot_hash() != snapshot.normalized_hash()
            || identity.principal() != snapshot.principal()
            || session.identity_hash != identity.normalized_hash()
            || session.issued_at != identity.issued_at()
            || session.expires_at != identity.expires_at()
            || snapshot.principal().kind != WorkloadKind::AgentInstance
            || snapshot.principal().id.is_nil()
            || snapshot.principal().revision_id.is_nil()
            || !matches!(identity.invocation(), RuntimeInvocation::Run(id) if id.as_uuid() == run)
        {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
        Ok(Self {
            run,
            session,
            snapshot,
            identity,
            generic_credential_hash,
            observed_at,
        })
    }
    /// Original Run UUID, distinct from the session identifier.
    #[must_use]
    pub const fn run(&self) -> Uuid {
        self.run
    }
    /// Full generic terminal row comparison metadata.
    #[must_use]
    pub const fn session(&self) -> &ExpiredRuntimeGitSessionMetadata {
        &self.session
    }
    /// Full generic immutable ceiling and bindings.
    #[must_use]
    pub const fn snapshot(&self) -> &AuthorizationSnapshot {
        &self.snapshot
    }
    /// Full generic immutable identity claims.
    #[must_use]
    pub const fn identity(&self) -> &RuntimeSessionIdentity {
        &self.identity
    }
    /// Redacted generic verifier to compare with the generic observation.
    #[must_use]
    pub const fn generic_credential_hash(&self) -> RuntimeCredentialHash {
        self.generic_credential_hash
    }
    /// Trusted observation instant; not a redelivery time override.
    #[must_use]
    pub const fn observed_at(&self) -> OffsetDateTime {
        self.observed_at
    }
}

/// An exact Git binding/verifier or positively qualified absence for one ceiling.
///
/// Absence construction is a representation, not a proof: only a trusted adapter
/// may qualify the full absent graph. No missing-session success is represented.
#[derive(Clone, PartialEq, Eq)]
pub struct ExpiredRuntimeGitHandoff {
    ceiling: ExpiredRuntimeGitCeiling,
    credential: Option<StoredRuntimeGitCredential>,
    credential_hash: Option<[u8; 32]>,
}

impl ExpiredRuntimeGitHandoff {
    /// Checks one exact immutable Git binding against its expired generic ceiling.
    ///
    /// # Errors
    /// Rejects mismatched session/snapshot/generation/expiry or nil binding/repository.
    pub fn present(
        ceiling: ExpiredRuntimeGitCeiling,
        credential: StoredRuntimeGitCredential,
        credential_hash: [u8; 32],
    ) -> Result<Self, RuntimeGitAuthorityError> {
        let session = ceiling.session();
        if credential.runtime_session_id != session.session
            || credential.authorization_snapshot_id != session.snapshot
            || credential.generation != session.generation
            || credential.expires_at != session.expires_at
            || credential.binding_id.is_nil()
            || credential.repository_id.as_uuid().is_nil()
        {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
        Ok(Self {
            ceiling,
            credential: Some(credential),
            credential_hash: Some(credential_hash),
        })
    }
    /// Represents independently qualified absence of the entire Git binding graph.
    ///
    /// The adapter must prove absence; a host store must deny any unexpected exact
    /// file rather than delete it. This never represents an absent generic session.
    #[must_use]
    pub const fn absent(ceiling: ExpiredRuntimeGitCeiling) -> Self {
        Self {
            ceiling,
            credential: None,
            credential_hash: None,
        }
    }
    /// Generic ceiling that must match the generic issuer's observed original.
    #[must_use]
    pub const fn ceiling(&self) -> &ExpiredRuntimeGitCeiling {
        &self.ceiling
    }
    /// Exact immutable Git binding, or positively qualified absent graph.
    #[must_use]
    pub const fn credential(&self) -> Option<&StoredRuntimeGitCredential> {
        self.credential.as_ref()
    }
    /// Durable Git verifier bytes for local authentication; never bearer bytes.
    #[must_use]
    pub const fn credential_hash(&self) -> Option<&[u8; 32]> {
        self.credential_hash.as_ref()
    }
    /// Exact request equality; no UUID identity inference or fallback.
    #[must_use]
    pub fn matches_request(
        &self,
        run: Uuid,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> bool {
        self.ceiling.run == run
            && self.ceiling.session.session == session
            && self.ceiling.session.generation == generation
    }
}

impl fmt::Debug for ExpiredRuntimeGitHandoff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExpiredRuntimeGitHandoff")
            .field("ceiling", &self.ceiling)
            .field("credential", &self.credential)
            .field("credential_hash", &"[REDACTED]")
            .finish()
    }
}

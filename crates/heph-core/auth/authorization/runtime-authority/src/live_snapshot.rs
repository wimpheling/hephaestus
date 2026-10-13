//! Read-only live run authority checks, separate from credential issuance.

use async_trait::async_trait;
use capability_domain::{
    AuthorityHash, AuthorizationSnapshot, AuthorizationSnapshotId, RuntimeCredentialGeneration,
    RuntimeSessionId, RuntimeSessionStatus,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{RuntimeAuthorityError, StoredRuntimeSession};

/// Exact frozen consumer and software pins; constructing these grants no authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveRunSnapshotPins {
    run: Uuid,
    instance: Uuid,
    revision: Uuid,
    release: Uuid,
    release_agent: Uuid,
    command: Uuid,
}

impl LiveRunSnapshotPins {
    /// Checks the complete immutable run identity without consulting current revisions.
    ///
    /// # Errors
    ///
    /// Rejects nil identifiers. The repository must prove all stored equality.
    pub fn new(
        run: Uuid,
        instance: Uuid,
        revision: Uuid,
        release: Uuid,
        release_agent: Uuid,
        command: Uuid,
    ) -> Result<Self, RuntimeAuthorityError> {
        if [run, instance, revision, release, release_agent, command]
            .iter()
            .any(Uuid::is_nil)
        {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        Ok(Self {
            run,
            instance,
            revision,
            release,
            release_agent,
            command,
        })
    }
    /// Exact run identifier.
    #[must_use]
    pub const fn run(self) -> Uuid {
        self.run
    }
    /// Exact instance identifier.
    #[must_use]
    pub const fn instance(self) -> Uuid {
        self.instance
    }
    /// Frozen instance revision.
    #[must_use]
    pub const fn revision(self) -> Uuid {
        self.revision
    }
    /// Frozen release identifier.
    #[must_use]
    pub const fn release(self) -> Uuid {
        self.release
    }
    /// Frozen released export identifier.
    #[must_use]
    pub const fn release_agent(self) -> Uuid {
        self.release_agent
    }
    /// Immutable producing command identifier.
    #[must_use]
    pub const fn command(self) -> Uuid {
        self.command
    }
}

/// Expected immutable facts loaded from an already persisted session and ceiling.
///
/// Mutable acknowledgement/status is deliberately excluded. This value is a
/// comparison request, not authentication or physical execution evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveRunSessionCeiling {
    session: RuntimeSessionId,
    snapshot: AuthorizationSnapshotId,
    identity_hash: AuthorityHash,
    snapshot_hash: AuthorityHash,
    generation: RuntimeCredentialGeneration,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
}

impl LiveRunSessionCeiling {
    /// Captures stable persisted metadata and the immutable snapshot hash.
    ///
    /// Use database-loaded timestamps, rather than uncommitted issuance input.
    ///
    /// # Errors
    ///
    /// Rejects nil session/snapshot identities and invalid validity intervals.
    pub fn new(
        stored: &StoredRuntimeSession,
        snapshot_hash: AuthorityHash,
    ) -> Result<Self, RuntimeAuthorityError> {
        if stored.id.as_uuid().is_nil()
            || stored.snapshot_id.as_uuid().is_nil()
            || stored.expires_at <= stored.issued_at
        {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        Ok(Self {
            session: stored.id,
            snapshot: stored.snapshot_id,
            identity_hash: stored.identity_hash,
            snapshot_hash,
            generation: stored.generation,
            issued_at: stored.issued_at,
            expires_at: stored.expires_at,
        })
    }

    /// Exact recorded session identifier.
    #[must_use]
    pub const fn session(self) -> RuntimeSessionId {
        self.session
    }
    /// Exact immutable ceiling digest.
    #[must_use]
    pub const fn snapshot_hash(self) -> AuthorityHash {
        self.snapshot_hash
    }

    /// Checks stable equality and fresh persisted lifecycle without requiring stale status equality.
    #[must_use]
    pub fn matches(
        self,
        stored: &StoredRuntimeSession,
        snapshot_hash: AuthorityHash,
        now: OffsetDateTime,
    ) -> bool {
        self.session == stored.id
            && self.snapshot == stored.snapshot_id
            && self.identity_hash == stored.identity_hash
            && self.snapshot_hash == snapshot_hash
            && self.generation == stored.generation
            && self.issued_at == stored.issued_at
            && self.expires_at == stored.expires_at
            && now >= stored.issued_at
            && now < stored.expires_at
            && stored.revoked_at.is_none()
            && matches!(
                stored.status,
                RuntimeSessionStatus::PendingHandoff | RuntimeSessionStatus::Active
            )
    }
}

/// Read-only pre-issuance or already-issued source/ceiling check.
///
/// Issued checks may survive intentional acquisition closure during owned drain;
/// they authorize no acquisition, Start, renewal, or new credential issuance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveRunSnapshotRequest {
    pins: LiveRunSnapshotPins,
    ceiling: Option<LiveRunSessionCeiling>,
}
impl LiveRunSnapshotRequest {
    /// Checks source and generic ceiling before issuance, without creating a session.
    #[must_use]
    pub const fn pre_issuance(pins: LiveRunSnapshotPins) -> Self {
        Self {
            pins,
            ceiling: None,
        }
    }
    /// Requires an already issued exact session and immutable ceiling.
    ///
    /// # Errors
    ///
    /// Rejects a session or snapshot belonging to another run.
    pub fn issued(
        pins: LiveRunSnapshotPins,
        ceiling: LiveRunSessionCeiling,
    ) -> Result<Self, RuntimeAuthorityError> {
        if ceiling.session.as_uuid() != pins.run || ceiling.snapshot.as_uuid() != pins.run {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        Ok(Self {
            pins,
            ceiling: Some(ceiling),
        })
    }
    /// Complete exact consumer pins.
    #[must_use]
    pub const fn pins(self) -> LiveRunSnapshotPins {
        self.pins
    }
    /// Existing immutable session ceiling, absent only for read-only pre-issuance.
    #[must_use]
    pub const fn ceiling(&self) -> Option<&LiveRunSessionCeiling> {
        self.ceiling.as_ref()
    }
}

/// Trusted worker read-only resolver; mount grants and caller launch rights remain separate.
#[async_trait]
pub trait LiveRunSnapshotResolver: Send + Sync {
    /// Resolves a fresh ceiling and verifies live source/generic operations.
    ///
    /// # Errors
    ///
    /// Unsupported profiles, unknown provenance, identity changes, revoked
    /// authority, unavailable storage and session mismatch fail closed.
    async fn resolve_live_snapshot(
        &self,
        request: &LiveRunSnapshotRequest,
        authorization_model_version: &str,
        now: OffsetDateTime,
    ) -> Result<AuthorizationSnapshot, RuntimeAuthorityError>;
}

#[cfg(test)]
#[path = "live_snapshot_tests.rs"]
mod tests;

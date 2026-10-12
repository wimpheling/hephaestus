//! Immutable comparison data for exact expired handoff deletion, never authority.

use crate::{LiveRunSnapshotPins, RuntimeAuthorityError, StoredRuntimeSession};
use capability_domain::{
    AuthorizationSnapshot, RuntimeCredentialGeneration, RuntimeCredentialHash, RuntimeInvocation,
    RuntimeSessionId, RuntimeSessionIdentity, RuntimeSessionStatus, WorkloadKind,
};
use time::OffsetDateTime;
use uuid::Uuid;

#[cfg(test)]
#[path = "expired/tests.rs"]
mod tests;

/// One internally coherent expired original supplied by a trusted repository.
///
/// Construction checks equality and shape only. The adapter must independently
/// prove original producer, owner/profile and stored graph under its worker scope.
/// This value grants no authorization, dispatch, deletion or physical receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredRuntimeHandoff {
    session: StoredRuntimeSession,
    snapshot: AuthorizationSnapshot,
    identity: RuntimeSessionIdentity,
    pins: LiveRunSnapshotPins,
    attachment: Option<Uuid>,
    credential_hash: RuntimeCredentialHash,
    observed_at: OffsetDateTime,
}

impl ExpiredRuntimeHandoff {
    /// Checks complete immutable claims against an observed expired session.
    ///
    /// `pins` is used only as immutable original comparison data; no live resolver
    /// or current actor check is performed. Use actual stored timestamps/hashes.
    ///
    /// # Errors
    /// Rejects non-expired, inconsistent, nil or malformed lifecycle metadata.
    pub fn new(
        session: StoredRuntimeSession,
        snapshot: AuthorizationSnapshot,
        identity: RuntimeSessionIdentity,
        pins: LiveRunSnapshotPins,
        attachment: Option<Uuid>,
        credential_hash: RuntimeCredentialHash,
        observed_at: OffsetDateTime,
    ) -> Result<Self, RuntimeAuthorityError> {
        let principal = snapshot.principal();
        if session.status != RuntimeSessionStatus::Expired
            || session.id.as_uuid().is_nil()
            || session.snapshot_id.as_uuid().is_nil()
            || attachment.is_some_and(|id| id.is_nil())
            || session.expires_at <= session.issued_at
            || observed_at < session.expires_at
            || session.revoked_at.is_some()
            || session
                .acknowledged_at
                .is_some_and(|at| at < session.issued_at || at >= session.expires_at)
            || session.id != identity.id()
            || session.snapshot_id != snapshot.id()
            || identity.snapshot_id() != snapshot.id()
            || identity.snapshot_hash() != snapshot.normalized_hash()
            || identity.principal() != principal
            || session.identity_hash != identity.normalized_hash()
            || session.issued_at != identity.issued_at()
            || session.expires_at != identity.expires_at()
            || principal.kind != WorkloadKind::AgentInstance
            || principal.id != pins.instance()
            || principal.revision_id != pins.revision()
            || !matches!(identity.invocation(), RuntimeInvocation::Run(run) if run.as_uuid() == pins.run())
        {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        Ok(Self {
            session,
            snapshot,
            identity,
            pins,
            attachment,
            credential_hash,
            observed_at,
        })
    }

    /// Observed terminal metadata, including immutable generation and validity.
    #[must_use]
    pub const fn session(&self) -> &StoredRuntimeSession {
        &self.session
    }
    /// Full immutable authority ceiling and bindings; not live authorization.
    #[must_use]
    pub const fn snapshot(&self) -> &AuthorizationSnapshot {
        &self.snapshot
    }
    /// Full immutable identity claims whose normalized digest was compared.
    #[must_use]
    pub const fn identity(&self) -> &RuntimeSessionIdentity {
        &self.identity
    }
    /// Complete original Run/software pins, without a current-state requirement.
    #[must_use]
    pub const fn pins(&self) -> LiveRunSnapshotPins {
        self.pins
    }
    /// Original optional repository attachment.
    #[must_use]
    pub const fn attachment(&self) -> Option<Uuid> {
        self.attachment
    }
    /// Redacted durable verifier for host-only envelope authentication.
    #[must_use]
    pub const fn credential_hash(&self) -> RuntimeCredentialHash {
        self.credential_hash
    }
    /// Trusted observation instant, never an invented redelivery time.
    #[must_use]
    pub const fn observed_at(&self) -> OffsetDateTime {
        self.observed_at
    }

    /// Compares the exact request without inferring session identity from a Run UUID.
    #[must_use]
    pub fn matches_request(
        &self,
        run: Uuid,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> bool {
        self.pins.run() == run
            && self.session.id == session
            && self.session.generation == generation
    }
}

//! Controlled Core comparison tests; no database/native provenance claims.

use super::*;
use capability_domain::{AuthorizationSnapshotId, WorkloadPrincipal};

#[path = "tests/ports.rs"]
mod ports;

struct Fixture {
    stored: StoredRuntimeSession,
    snapshot: AuthorizationSnapshot,
    identity: RuntimeSessionIdentity,
    pins: LiveRunSnapshotPins,
    verifier: RuntimeCredentialHash,
    observed: OffsetDateTime,
}
impl Fixture {
    fn new(run: Uuid) -> Self {
        let principal = WorkloadPrincipal::new(
            WorkloadKind::AgentInstance,
            Uuid::from_u128(2),
            Uuid::from_u128(3),
        );
        let snapshot = AuthorizationSnapshot::new(
            AuthorizationSnapshotId::from_uuid(Uuid::from_u128(7)),
            principal,
            "controlled-model",
            vec![],
        )
        .unwrap();
        let issued = OffsetDateTime::UNIX_EPOCH;
        let expires = issued + time::Duration::seconds(60);
        // Session differs from Run: this must not become a UUID identity fallback.
        let identity = RuntimeSessionIdentity::new(
            RuntimeSessionId::from_uuid(Uuid::from_u128(8)),
            principal,
            RuntimeInvocation::Run(run.into()),
            &snapshot,
            issued,
            expires,
        )
        .unwrap();
        let stored = StoredRuntimeSession {
            id: identity.id(),
            snapshot_id: snapshot.id(),
            identity_hash: identity.normalized_hash(),
            generation: RuntimeCredentialGeneration::INITIAL,
            status: RuntimeSessionStatus::Expired,
            issued_at: issued,
            expires_at: expires,
            acknowledged_at: Some(issued + time::Duration::seconds(1)),
            revoked_at: None,
        };
        Self {
            stored,
            snapshot,
            identity,
            pins: LiveRunSnapshotPins::new(
                run,
                principal.id,
                principal.revision_id,
                Uuid::from_u128(4),
                Uuid::from_u128(5),
                Uuid::from_u128(6),
            )
            .unwrap(),
            verifier: RuntimeCredentialHash::from_bytes([0xa7; 32]),
            observed: expires,
        }
    }
    fn projection(self) -> Result<ExpiredRuntimeHandoff, RuntimeAuthorityError> {
        ExpiredRuntimeHandoff::new(
            self.stored,
            self.snapshot,
            self.identity,
            self.pins,
            Some(Uuid::from_u128(9)),
            self.verifier,
            self.observed,
        )
    }
}

#[test]
fn coherent_expired_original_preserves_full_metadata_without_live_authority() {
    let f = Fixture::new(Uuid::from_u128(1));
    let stored = f.stored.clone();
    let snapshot = f.snapshot.clone();
    let identity = f.identity.clone();
    let pins = f.pins;
    let verifier = f.verifier;
    let original = f.projection().unwrap();
    assert_eq!(original.session(), &stored);
    assert_eq!(original.snapshot(), &snapshot);
    assert_eq!(original.identity(), &identity);
    assert_eq!(original.pins(), pins);
    assert_eq!(original.attachment(), Some(Uuid::from_u128(9)));
    assert_eq!(original.credential_hash(), verifier);
    assert_eq!(original.observed_at(), stored.expires_at);
    assert!(!original.identity().is_valid_at(original.observed_at()));
    assert!(original.matches_request(pins.run(), stored.id, stored.generation));
    assert!(!original.matches_request(stored.id.as_uuid(), stored.id, stored.generation));
    assert!(!original.matches_request(
        pins.run(),
        stored.id,
        RuntimeCredentialGeneration::new(2).unwrap()
    ));
    assert!(!format!("{original:?}").contains("167, 167"));
}

#[test]
fn active_pending_revoked_and_malformed_terminal_shapes_deny() {
    for status in [
        RuntimeSessionStatus::Active,
        RuntimeSessionStatus::PendingHandoff,
        RuntimeSessionStatus::Revoked,
    ] {
        let mut f = Fixture::new(Uuid::from_u128(1));
        f.stored.status = status;
        assert_eq!(
            f.projection().unwrap_err(),
            RuntimeAuthorityError::IdentityMismatch
        );
    }
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.stored.revoked_at = Some(f.stored.issued_at);
    assert!(f.projection().is_err());
    for at in [
        OffsetDateTime::UNIX_EPOCH - time::Duration::seconds(1),
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60),
    ] {
        let mut f = Fixture::new(Uuid::from_u128(1));
        f.stored.acknowledged_at = Some(at);
        assert!(f.projection().is_err());
    }
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.observed -= time::Duration::seconds(1);
    assert!(f.projection().is_err());
}

#[test]
fn session_snapshot_identity_and_original_substitutions_deny() {
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.stored.id = RuntimeSessionId::from_uuid(Uuid::from_u128(99));
    assert!(f.projection().is_err());
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.stored.snapshot_id = AuthorizationSnapshotId::from_uuid(Uuid::from_u128(99));
    assert!(f.projection().is_err());
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.stored.identity_hash = capability_domain::AuthorityHash::from_bytes([0; 32]);
    assert!(f.projection().is_err());
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.pins = LiveRunSnapshotPins::new(
        Uuid::from_u128(99),
        f.pins.instance(),
        f.pins.revision(),
        f.pins.release(),
        f.pins.release_agent(),
        f.pins.command(),
    )
    .unwrap();
    assert!(f.projection().is_err());
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.snapshot = AuthorizationSnapshot::new(
        f.snapshot.id(),
        f.snapshot.principal(),
        "changed-ceiling",
        vec![],
    )
    .unwrap();
    assert!(f.projection().is_err());
}

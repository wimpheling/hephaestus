use capability_domain::{
    AuthorityHash, AuthorizationSnapshotId, RuntimeCredentialGeneration, RuntimeSessionId,
    RuntimeSessionStatus,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{LiveRunSessionCeiling, LiveRunSnapshotPins, LiveRunSnapshotRequest};
use crate::StoredRuntimeSession;

fn session(run: Uuid) -> StoredRuntimeSession {
    StoredRuntimeSession {
        id: RuntimeSessionId::from_uuid(run),
        snapshot_id: AuthorizationSnapshotId::from_uuid(run),
        identity_hash: AuthorityHash::from_bytes([1; 32]),
        generation: RuntimeCredentialGeneration::INITIAL,
        status: RuntimeSessionStatus::PendingHandoff,
        issued_at: OffsetDateTime::UNIX_EPOCH,
        expires_at: OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
        acknowledged_at: None,
        revoked_at: None,
    }
}
fn pins(run: Uuid) -> LiveRunSnapshotPins {
    LiveRunSnapshotPins::new(
        run,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    )
    .unwrap()
}

#[test]
fn pre_issuance_has_no_session_and_nil_pins_are_rejected() {
    let requested = pins(Uuid::new_v4());
    let request = LiveRunSnapshotRequest::pre_issuance(requested);
    assert_eq!(request.pins(), requested);
    assert!(request.ceiling().is_none());
    assert!(
        LiveRunSnapshotPins::new(
            Uuid::nil(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4()
        )
        .is_err()
    );
}

#[test]
fn issued_requests_cannot_borrow_another_runs_session() {
    let recorded = session(Uuid::new_v4());
    let ceiling =
        LiveRunSessionCeiling::new(&recorded, AuthorityHash::from_bytes([2; 32])).unwrap();
    assert!(LiveRunSnapshotRequest::issued(pins(recorded.id.as_uuid()), ceiling).is_ok());
    assert!(LiveRunSnapshotRequest::issued(pins(Uuid::new_v4()), ceiling).is_err());
    let mut wrong_snapshot = recorded;
    wrong_snapshot.snapshot_id = AuthorizationSnapshotId::new();
    let ceiling =
        LiveRunSessionCeiling::new(&wrong_snapshot, AuthorityHash::from_bytes([2; 32])).unwrap();
    assert!(LiveRunSnapshotRequest::issued(pins(wrong_snapshot.id.as_uuid()), ceiling).is_err());
}

#[test]
fn fresh_acknowledgement_does_not_change_an_issued_ceiling() {
    let mut recorded = session(Uuid::new_v4());
    let hash = AuthorityHash::from_bytes([2; 32]);
    let ceiling = LiveRunSessionCeiling::new(&recorded, hash).unwrap();
    recorded.status = RuntimeSessionStatus::Active;
    recorded.acknowledged_at = Some(recorded.issued_at + time::Duration::seconds(1));
    assert!(ceiling.matches(
        &recorded,
        hash,
        recorded.issued_at + time::Duration::seconds(2)
    ));
}

#[test]
fn revocation_expiry_and_stable_fact_substitution_fail_closed() {
    let original = session(Uuid::new_v4());
    let hash = AuthorityHash::from_bytes([2; 32]);
    let ceiling = LiveRunSessionCeiling::new(&original, hash).unwrap();
    assert!(!ceiling.matches(&original, hash, original.expires_at));
    assert!(!ceiling.matches(
        &original,
        AuthorityHash::from_bytes([3; 32]),
        original.issued_at
    ));
    let mut changed = original.clone();
    changed.status = RuntimeSessionStatus::Revoked;
    changed.revoked_at = Some(changed.issued_at);
    assert!(!ceiling.matches(&changed, hash, changed.issued_at));
    changed = original.clone();
    changed.generation = RuntimeCredentialGeneration::new(2).unwrap();
    assert!(!ceiling.matches(&changed, hash, changed.issued_at));
    changed = original.clone();
    changed.identity_hash = AuthorityHash::from_bytes([9; 32]);
    assert!(!ceiling.matches(&changed, hash, changed.issued_at));
    changed = original;
    changed.expires_at += time::Duration::seconds(1);
    assert!(!ceiling.matches(&changed, hash, changed.issued_at));
}

use capability_domain::{
    AuthorityHash, AuthorizationSnapshotId, RuntimeCredentialGeneration, RuntimeSessionId,
    RuntimeSessionStatus,
};
use runtime_authority::StoredRuntimeSession;
use uuid::Uuid;

use super::*;

fn pins() -> LiveRunSnapshotPins {
    LiveRunSnapshotPins::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    )
    .unwrap()
}

#[test]
fn absent_session_never_falls_back_after_preissuance() {
    let pins = pins();
    for state in [RunState::LeasingVolume, RunState::Provisioning] {
        assert!(request(pins, state, None).unwrap().ceiling().is_none());
    }
    for state in [
        RunState::Queued,
        RunState::Starting,
        RunState::Running,
        RunState::Succeeded,
        RunState::Failed,
        RunState::CleaningUp,
        RunState::CleanedUp,
    ] {
        assert!(request(pins, state, None).is_err(), "{state:?}");
    }
}

#[test]
fn recorded_ceiling_is_retained_even_if_phase_could_preissue() {
    let pins = pins();
    let now = OffsetDateTime::now_utc();
    let stored = StoredRuntimeSession {
        id: RuntimeSessionId::from_uuid(pins.run()),
        snapshot_id: AuthorizationSnapshotId::from_uuid(pins.run()),
        identity_hash: AuthorityHash::from_bytes([11; 32]),
        generation: RuntimeCredentialGeneration::INITIAL,
        status: RuntimeSessionStatus::Revoked,
        issued_at: now - time::Duration::seconds(1),
        expires_at: now + time::Duration::minutes(1),
        acknowledged_at: None,
        revoked_at: Some(now),
    };
    let ceiling = LiveRunSessionCeiling::new(&stored, AuthorityHash::from_bytes([12; 32])).unwrap();
    let selected = request(pins, RunState::Provisioning, Some(ceiling)).unwrap();
    assert_eq!(selected.ceiling(), Some(&ceiling));
    assert!(!ceiling.matches(&stored, ceiling.snapshot_hash(), now));
}

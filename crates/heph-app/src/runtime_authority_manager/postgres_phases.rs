use capability_domain::RuntimeSessionId;
use heph_run::{RunAuthorityManager, RunState};
use runtime_authority::{LiveRunSnapshotPins, RuntimeSessionRepository};
use sqlx::PgPool;

use super::fixture::{self, Fixture};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn actual_manager_issues_only_in_provisioning_and_checks_pending_active_and_drain(
    pool: PgPool,
) {
    let mut f = Fixture::new(pool).await;
    f.manager.reauthorize(&f.run).await.unwrap();
    assert!(f.manager.prepare(&f.run).await.is_err());
    assert!(f.evidence().await["session"].is_null());
    assert!(f.envelopes().is_empty());
    f.advance(RunState::Provisioning).await;
    let prepared = f.manager.prepare(&f.run).await.unwrap();
    assert!(prepared.bootstrap.is_some());
    drop(prepared);
    let pending = f.evidence().await;
    assert_eq!(pending["session"]["status"], "pending_handoff");
    let encrypted = f.envelopes();
    assert!(!encrypted.is_empty());
    f.manager.reauthorize(&f.run).await.unwrap();
    assert_eq!(f.evidence().await, pending);
    assert_eq!(f.envelopes(), encrypted);
    let pins = pins(&f);
    let ceiling = f
        .manager
        .repository
        .recorded_live_session_ceiling(pins)
        .await
        .unwrap()
        .unwrap();
    f.advance(RunState::Starting).await;
    f.manager.reauthorize(&f.run).await.unwrap();
    let session = f
        .manager
        .repository
        .find(RuntimeSessionId::from_uuid(f.run.id.as_uuid()))
        .await
        .unwrap()
        .unwrap();
    f.manager
        .acknowledge(&f.run, session.id.as_uuid(), session.generation.get())
        .await
        .unwrap();
    let active = f.evidence().await;
    assert_eq!(active["session"]["status"], "active");
    assert!(f.envelopes().is_empty());
    assert_eq!(
        f.manager
            .repository
            .recorded_live_session_ceiling(pins)
            .await
            .unwrap(),
        Some(ceiling)
    );
    for phase in [RunState::Running, RunState::Succeeded, RunState::CleaningUp] {
        f.advance(phase).await;
        f.manager.reauthorize(&f.run).await.unwrap();
        assert!(f.manager.prepare(&f.run).await.is_err());
        assert_eq!(f.evidence().await, active);
        assert!(f.envelopes().is_empty());
    }
    f.worker.close().await;
    println!(
        "REAL_CANONICAL_AUTHORITY_MANAGER actual_issue=1 encrypted_handoff=1 pending_active=1 live_phases=1 closed_drain=1 readonly=1 no_vm=1"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn absent_session_after_start_and_default_manager_are_denied(pool: PgPool) {
    let mut f = Fixture::new(pool).await;
    let legacy = fixture::manager(&f.worker, &f.root, false);
    assert!(legacy.reauthorize(&f.run).await.is_err());
    f.advance(RunState::Provisioning).await;
    f.advance(RunState::Starting).await;
    assert!(f.manager.reauthorize(&f.run).await.is_err());
    assert!(legacy.reauthorize(&f.run).await.is_err());
    assert!(f.manager.prepare(&f.run).await.is_err());
    assert!(f.evidence().await["session"].is_null());
    assert!(f.envelopes().is_empty());
    drop(legacy);
    f.worker.close().await;
}

pub fn pins(f: &Fixture) -> LiveRunSnapshotPins {
    LiveRunSnapshotPins::new(
        f.run.id.as_uuid(),
        f.run.instance_id.as_uuid(),
        f.run.instance_revision_id.as_uuid(),
        f.run.release_id.as_uuid(),
        f.run.release_agent_id.as_uuid(),
        f.run.command_id.as_uuid(),
    )
    .unwrap()
}

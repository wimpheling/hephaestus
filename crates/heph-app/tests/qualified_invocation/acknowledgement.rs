use capability_domain::RuntimeSessionStatus;
use heph_run::{RunAuthorityManager, RunRepository, RunState};
use run_orchestrator::RunLaunchAuthorizer;
use sqlx::PgPool;
use uuid::Uuid;

use super::fixture::Fixture;

async fn exact_ack_and_attach(pool: &PgPool, count: usize) {
    let f = Box::pin(Fixture::provisioning(pool, count)).await;
    let run = f.run().await;
    let bootstrap = f.issue().await;
    let before = f.rows().await;
    let handoff = f.handoff_files();
    let manager = f.manager();
    reject_mismatched_acknowledgements(&f, &run, &manager, &before, &handoff).await;
    manager
        .acknowledge(&run, bootstrap.session_id(), bootstrap.generation())
        .await
        .unwrap();
    let active = f.session().await;
    assert_eq!(active.status, RuntimeSessionStatus::Active);
    assert!(active.acknowledged_at.is_some());
    assert!(f.handoff_files().is_empty());
    let acknowledged = f.rows().await;
    manager
        .acknowledge(&run, bootstrap.session_id(), bootstrap.generation())
        .await
        .unwrap();
    assert_eq!(f.session().await, active);
    assert_eq!(f.rows().await, acknowledged);
    if count != 0 {
        assert!(
            f.birth
                .repository()
                .transition(run.id, RunState::Starting, None, None)
                .await
                .is_err(),
            "ACK alone cannot claim the full attached set"
        );
        assert_eq!(f.rows().await, acknowledged);
    }
    f.attach().await;
    authorize_attached_late_phases(&f, &run, &manager, &active).await;
    f.birth.physical.assert_unchanged();
}
async fn reject_mismatched_acknowledgements(
    f: &Fixture,
    run: &heph_run::Run,
    manager: &hephaestus_app::PgRunAuthorityManager,
    before: &serde_json::Value,
    handoff: &std::collections::BTreeMap<String, String>,
) {
    for (id, generation) in [
        (Uuid::new_v4(), 1),
        (run.id.as_uuid(), 0),
        (run.id.as_uuid(), 2),
    ] {
        assert!(manager.acknowledge(run, id, generation).await.is_err());
        assert_eq!(f.rows().await, *before);
        assert_eq!(f.handoff_files(), *handoff);
    }
    assert!(
        f.birth
            .repository()
            .transition(run.id, RunState::Starting, None, None)
            .await
            .is_err(),
        "starting requires acknowledged exact session"
    );
    assert_eq!(f.rows().await, *before);
}

async fn authorize_attached_late_phases(
    f: &Fixture,
    run: &heph_run::Run,
    manager: &hephaestus_app::PgRunAuthorityManager,
    active: &runtime_authority::StoredRuntimeSession,
) {
    for phase in [RunState::Starting, RunState::Running] {
        let current = f
            .birth
            .repository()
            .transition(run.id, phase, None, None)
            .await
            .unwrap();
        assert_eq!(current, f.run().await);
        let before = f.rows().await;
        manager.reauthorize(&current).await.unwrap();
        f.launcher().authorize(&current).await.unwrap();
        f.launcher().authorize_with_volumes(&current).await.unwrap();
        assert!(manager.prepare(&current).await.is_err());
        assert!(
            manager
                .acknowledge(&current, run.id.as_uuid(), 1)
                .await
                .is_err()
        );
        assert_eq!(f.session().await, *active);
        assert_eq!(f.rows().await, before);
        assert!(f.handoff_files().is_empty());
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_zero_selection_ack_is_exact_and_late_phases_do_not_issue(pool: PgPool) {
    exact_ack_and_attach(&pool, 0).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_two_selection_ack_cannot_replace_complete_attached_fences(pool: PgPool) {
    exact_ack_and_attach(&pool, 2).await;
}

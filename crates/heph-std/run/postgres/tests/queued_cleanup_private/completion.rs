use super::{
    controller::{Control, build},
    observations::{rows, unconsumed},
    queued_fixture::Queued,
};
use run_orchestrator::{RunCleanupRepository, RunCleanupWakeProgress, RunRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn actual_core_zero_set_scoped_receipt_completion_and_exact_replay(pool: PgPool) {
    let h = Queued::new(&pool, 0, true).await;
    let prepared = h.prepare().await;
    let control = Control::new();
    let core = build(&h, control.clone(), None, None);
    assert_eq!(
        core.clone().wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(
        *control.log.lock().unwrap(),
        vec!["scoped-entered", "scoped-confirmed", "completion"]
    );
    let terminal = h.read().await;
    assert_eq!(terminal.binding, prepared.binding);
    assert_eq!(terminal.run.state, run_domain::RunState::CleanedUp);
    assert_eq!(
        terminal.run.outcome,
        Some(run_domain::RunOutcome::Cancelled)
    );
    assert!(terminal.run.exit.is_none());
    assert!(terminal.run.failure.is_none());
    let receipt = h
        .cleanup()
        .recorded_cleanup_receipt(h.request.run_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.target(), &h.target().await);
    assert!(receipt.target().leases().is_empty());
    assert_eq!(
        h.repository().get(h.request.run_id()).await.unwrap(),
        terminal.run
    );
    let stable = rows(&pool).await;
    assert_eq!(
        h.cleanup().finish_cleanup(&receipt).await.unwrap(),
        terminal.run
    );
    assert_eq!(
        core.wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(control.count("scoped-confirmed"), 1);
    assert_eq!(rows(&pool).await, stable);
    unconsumed(&h).await;
    h.fixture.birth.assert_historical_migrations(&pool).await;
}

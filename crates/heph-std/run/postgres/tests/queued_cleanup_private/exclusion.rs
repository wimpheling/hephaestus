//! Actual same-controller Start claim and operation mutex; controlled inventory.
use super::{
    controller::{Control, build},
    controller_repository::{Forward, Gate},
    observations::rows,
    queued_fixture::Queued,
};
use run_orchestrator::{OrchestratorError, RunCleanupWakeProgress, RunRepository};
use sqlx::PgPool;
use std::sync::{Arc, atomic::Ordering};

#[sqlx::test(migrations = false)]
async fn same_controller_start_claim_and_operation_mutex_exclude_cleanup_callbacks(pool: PgPool) {
    let h = Queued::new(&pool, 0, true).await;
    h.prepare().await;
    let gate = Gate::new();
    let control = Control::new();
    let repository = Arc::new(Forward {
        inner: h.repository(),
        start: Some(gate.clone()),
        recovery: None,
    });
    let core = build(&h, control.clone(), Some(repository), None);
    let before = rows(&pool).await;
    let starting = core.clone();
    let command = h.fixture.start.clone();
    let start = tokio::spawn(async move { starting.start_run(&command).await });
    gate.wait().await;
    assert_eq!(
        core.wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Held
    );
    assert_eq!(rows(&pool).await, before);
    assert!(control.log.lock().unwrap().is_empty());
    gate.release.add_permits(1);
    let error = start.await.unwrap().unwrap_err();
    let OrchestratorError::Repository(error) = error else {
        panic!("actual delayed Start: {error:?}")
    };
    super::execution::repository_code(&error, "P1171");
    assert_eq!(rows(&pool).await, before);
    let control = Control::new();
    control.pause.store(true, Ordering::SeqCst);
    let cleanup = super::cleanup_port::Forward::new(h.cleanup());
    let repository = Arc::new(Forward {
        inner: h.repository(),
        start: None,
        recovery: Some(h.request.run_id()),
    });
    let core = build(&h, control.clone(), Some(repository), Some(cleanup.clone()));
    let waking = core.clone();
    let request = h.request.clone();
    let wake = tokio::spawn(async move { waking.wake_cleanup(&request).await });
    control.wait_entered().await;
    cleanup.wait_begins(2).await;
    assert_eq!(
        h.repository().get(h.request.run_id()).await.unwrap().state,
        run_domain::RunState::Cancelled
    );
    let paused = rows(&pool).await;
    let recovering = core.clone();
    let mut recovery = tokio::spawn(async move { recovering.recover_after_restart().await });
    cleanup.wait_begins(3).await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), &mut recovery)
            .await
            .is_err(),
        "actual second cleanup waits for the same operation mutex"
    );
    assert_eq!(control.count("scoped-entered"), 1);
    assert_eq!(rows(&pool).await, paused);
    recovery.abort();
    assert!(recovery.await.unwrap_err().is_cancelled());
    control.release.add_permits(1);
    assert_eq!(
        wake.await.unwrap().unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(control.count("scoped-confirmed"), 1);
    assert_eq!(control.count("completion"), 1);
    super::observations::unconsumed(&h).await;
    // Inventory was a controlled forwarding fixture with a real strict PG get.
    // Production recovery inventory support remains separate acceptance work.
}

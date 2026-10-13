//! Production orchestrator/PG adapters, controlled provider only.
use super::{base, fixture, provider::Provider, volumes::ControlledVolumes};
use async_trait::async_trait;
use run_domain::{LegacyVmPlacementScope, Run, RunOutcome, RunState};
use run_orchestrator::{RunCompletionError, RunCompletionObserver, RunOrchestrator, RunRepository};
use sqlx::PgPool;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use volume_trait::{ScalarLeaseHistory, VolumeMetadataRepository};
#[path = "../../../../../heph-core/runtime/run/orchestrator/tests/orchestrator/support/fakes.rs"]
// Only the fixed controlled workload spec is needed.
#[allow(dead_code)]
mod fakes;
#[path = "../../../../../heph-core/runtime/run/orchestrator/tests/orchestrator/support/helpers.rs"]
// Reuse controlled mutex helper; unrelated assertions are unused.
#[allow(dead_code)]
mod helpers;
#[path = "../../../../../heph-core/runtime/run/orchestrator/tests/orchestrator/support/vm.rs"]
// Actual core controlled provider behavior does not prove host IO.
#[allow(dead_code)]
pub mod vm;
struct Completion {
    pool: PgPool,
    log: Arc<Mutex<Vec<&'static str>>>,
}
#[async_trait]
impl RunCompletionObserver for Completion {
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError> {
        let exact:bool=sqlx::query_scalar("SELECT state='cleaned_up' AND volume_acquisition_closed_at IS NOT NULL AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL) FROM runs WHERE id=$1")
   .bind(run.id.as_uuid()).fetch_one(&self.pool).await.unwrap();
        assert!(exact);
        self.log.lock().unwrap().push("callback");
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunCompletionError> {
        panic!("strict mode cannot sweep callbacks")
    }
}
pub fn orchestrator(
    pool: &PgPool,
    f: &base::Fixture,
    owner: &LegacyVmPlacementScope,
    log: &Arc<Mutex<Vec<&'static str>>>,
    fail: &Arc<AtomicBool>,
) -> RunOrchestrator {
    RunOrchestrator::new(
        Arc::new(f.runs.clone()),
        Arc::new(ControlledVolumes {
            metadata: f.volumes.clone(),
            volume: f.volume,
            log: log.clone(),
        }),
        Arc::new(Provider {
            inner: vm::AutoExitProvider::new(log.clone()),
            scope: owner.provider_scope().clone(),
            fail: fail.clone(),
            log: log.clone(),
        }),
        Arc::new(fakes::TestSpecFactory),
        16_777_216,
    )
    .with_legacy_vm_placement(owner.clone())
    .with_completion_observer(Arc::new(Completion {
        pool: pool.clone(),
        log: log.clone(),
    }))
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private strict Legacy PostgreSQL"]
async fn stateful_core_orders_scoped_confirmation_release_and_callback(pool: PgPool) {
    fixture::schema(&pool).await;
    success(&pool, true).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private strict Legacy PostgreSQL"]
async fn stateless_core_uses_global_no_history_and_scoped_confirmation(pool: PgPool) {
    fixture::schema(&pool).await;
    success(&pool, false).await;
}
async fn success(pool: &PgPool, state: bool) {
    let owner = fixture::scope();
    let f = fixture::fresh(pool, state, &owner).await;
    let log = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(AtomicBool::new(false));
    let orch = orchestrator(pool, &f, &owner, &log, &fail);
    let done = orch.start_run(&f.command).await.unwrap();
    assert_eq!(done.state, RunState::CleanedUp);
    assert_eq!(done.outcome, Some(RunOutcome::Succeeded));
    let entries = log.lock().unwrap().clone();
    let confirmed = entries
        .iter()
        .position(|e| *e == "scoped-confirmed")
        .unwrap();
    let callback = entries.iter().position(|e| *e == "callback").unwrap();
    assert!(confirmed < callback);
    if state {
        let released = entries.iter().position(|e| *e == "release").unwrap();
        assert!(confirmed < released && released < callback);
    } else {
        assert!(!entries.contains(&"release"));
    }
    let before = log.lock().unwrap().len();
    let replay = orch.start_run(&f.command).await.unwrap();
    assert_eq!(replay, done);
    assert_eq!(&log.lock().unwrap()[before..], &["callback"]);
    assert_no_canonical_receipt(pool, done.id).await;
    f.worker.close().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private strict Legacy PostgreSQL"]
async fn persistent_destroy_denial_retains_known_exit_and_fence_then_restart_recovers(
    pool: PgPool,
) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, true, &owner).await;
    let log = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(AtomicBool::new(true));
    let orch = orchestrator(&pool, &f, &owner, &log, &fail);
    assert!(orch.start_run(&f.command).await.is_err());
    let held = f
        .volumes
        .scalar_lease_history(f.command.run_id)
        .await
        .unwrap();
    assert!(matches!(held, ScalarLeaseHistory::Held(_)));
    let row = f.runs.get(f.command.run_id).await.unwrap();
    assert_eq!(row.state, RunState::CleaningUp);
    assert_eq!(row.outcome, Some(RunOutcome::Failed));
    assert_eq!(row.exit.as_ref().and_then(|e| e.code), Some(0));
    {
        let entries = log.lock().unwrap();
        assert!(!entries.contains(&"callback"));
        assert!(!entries.contains(&"release"));
        drop(entries);
    }
    fail.store(false, Ordering::SeqCst);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert!(matches!(
        f.volumes.scalar_lease_history(row.id).await.unwrap(),
        ScalarLeaseHistory::Released(_)
    ));
    assert_eq!(f.runs.get(row.id).await.unwrap().exit, row.exit);
    assert_no_canonical_receipt(&pool, row.id).await;
    f.worker.close().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private strict Legacy PostgreSQL"]
async fn vm_null_committed_nonexpired_lease_restarts_without_uuid_history_fallback(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, true, &owner).await;
    let lease = super::evidence::acquire(&f, &owner).await;
    assert!(f.runs.get(f.command.run_id).await.unwrap().vm_id.is_none());
    assert!(lease.expires_at > time::OffsetDateTime::now_utc());
    let log = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(AtomicBool::new(false));
    let orch = orchestrator(&pool, &f, &owner, &log, &fail);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(
        f.runs.get(f.command.run_id).await.unwrap().state,
        RunState::CleanedUp
    );
    let entries = log.lock().unwrap().clone();
    assert!(!entries.contains(&"provision"));
    assert!(!entries.contains(&"start"));
    assert_eq!(
        entries.iter().filter(|e| **e == "scoped-confirmed").count(),
        1
    );
    assert!(entries.contains(&"release"));
    assert!(entries.contains(&"callback"));
    assert_no_canonical_receipt(&pool, f.command.run_id).await;
    f.worker.close().await;
}
async fn assert_no_canonical_receipt(pool: &PgPool, run: runtime_types::RunId) {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_cleanup_receipts WHERE run_id=$1")
            .bind(run.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

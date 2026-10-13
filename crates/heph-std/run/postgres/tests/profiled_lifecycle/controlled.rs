//! Production core and real PG ports; controlled VM, not kernel proof.

use async_trait::async_trait;
use run_domain::{Run, RunOutcome, RunState};
use run_orchestrator::{RunCompletionError, RunCompletionObserver, RunOrchestrator};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

use super::{
    controlled_volumes::ControlledVolumes,
    fixture::{self, Fixture},
};

#[path = "../../../../../heph-core/runtime/run/orchestrator/tests/orchestrator/support/fakes.rs"]
// Only its fixed workload factory is needed for these real adapter regressions.
#[allow(dead_code)]
mod fakes;
#[path = "../../../../../heph-core/runtime/run/orchestrator/tests/orchestrator/support/helpers.rs"]
// Reuse the controlled provider's mutex helper; other core assertions unused.
#[allow(dead_code)]
mod helpers;
#[path = "../../../../../heph-core/runtime/run/orchestrator/tests/orchestrator/support/vm.rs"]
// Reuse actual core controlled VM behavior, not physical host ownership proof.
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
        assert!(exact, "callback only after closed complete terminal commit");
        self.log.lock().unwrap().push("callback");
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunCompletionError> {
        Ok(0)
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn production_core_stateful_destroy_release_terminal_callback_order(pool: PgPool) {
    fixture::schema(&pool).await;
    controlled_success(&pool, true).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn production_core_stateless_destroy_precedes_terminal_callback(pool: PgPool) {
    fixture::schema(&pool).await;
    controlled_success(&pool, false).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn production_core_destroy_failure_keeps_cleaning_up_fence_and_skips_callback(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, true).await;
    let log = Arc::new(Mutex::new(Vec::new()));
    let volumes = Arc::new(ControlledVolumes {
        metadata: f.volumes.clone(),
        volume: f.volume,
        log: Arc::clone(&log),
    });
    let provider = Arc::new(super::failed_provider::FailedDestroyProvider {
        inner: vm::AutoExitProvider::new(Arc::clone(&log)),
    });
    let orchestrator = RunOrchestrator::new(
        Arc::new(f.runs.clone()),
        volumes,
        provider,
        Arc::new(fakes::TestSpecFactory),
        16_777_216,
    )
    .with_completion_observer(Arc::new(Completion {
        pool: pool.clone(),
        log: Arc::clone(&log),
    }));
    assert!(orchestrator.start_run(&f.command).await.is_err());
    let evidence = fixture::evidence(&pool, f.command.run_id).await;
    assert_eq!(evidence["run"]["state"], "cleaning_up");
    assert_eq!(evidence["run"]["outcome"], "failed");
    assert_eq!(evidence["run"]["exit_code"], 0);
    assert!(evidence["run"]["exit_signal"].is_null());
    assert!(!evidence["run"]["volume_acquisition_closed_at"].is_null());
    assert!(evidence["leases"][0]["released_at"].is_null());
    assert_eq!(
        evidence["leases"][0]["fencing_token"],
        evidence["volumes"][0]["lease_generation"]
    );
    {
        let events = log.lock().unwrap();
        assert!(!events.contains(&"release"));
        assert!(!events.contains(&"callback"));
        drop(events);
    }
    f.worker.close().await;
}

async fn controlled_success(pool: &PgPool, stateful: bool) {
    let f = Fixture::new(pool, stateful).await;
    let log = Arc::new(Mutex::new(Vec::new()));
    let volumes = Arc::new(ControlledVolumes {
        metadata: f.volumes.clone(),
        volume: f.volume,
        log: Arc::clone(&log),
    });
    let provider = Arc::new(vm::AutoExitProvider::new(Arc::clone(&log)));
    let orchestrator = RunOrchestrator::new(
        Arc::new(f.runs.clone()),
        volumes,
        provider,
        Arc::new(fakes::TestSpecFactory),
        16_777_216,
    )
    .with_completion_observer(Arc::new(Completion {
        pool: pool.clone(),
        log: Arc::clone(&log),
    }));
    let result = orchestrator.start_run(&f.command).await.unwrap();
    assert_eq!(result.state, RunState::CleanedUp);
    assert_eq!(result.outcome, Some(RunOutcome::Succeeded));
    let entries = log.lock().unwrap().clone();
    let destroyed = entries.iter().position(|e| *e == "destroy").unwrap();
    let callback = entries.iter().position(|e| *e == "callback").unwrap();
    assert!(destroyed < callback);
    if stateful {
        let released = entries.iter().position(|e| *e == "release").unwrap();
        assert!(destroyed < released && released < callback);
    }
    f.worker.close().await;
}

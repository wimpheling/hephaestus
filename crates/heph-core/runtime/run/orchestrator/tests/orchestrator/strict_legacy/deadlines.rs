use super::{super::support::lock, fixture::Fixture};
use async_trait::async_trait;
use run_domain::{Run, RunState};
use run_orchestrator::{PreparedRunRuntime, RunRuntimeError, RunRuntimeManager};
use runtime_types::RunId;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use volume_trait::ScalarLeaseHistory;

#[tokio::test(start_paused = true)]
async fn hanging_closure_is_bounded_and_exact_physical_fallback_quarantines_future_io() {
    let f = Fixture::new(true);
    f.runs.hang_close.store(true, Ordering::SeqCst);
    let orch = f.orchestrator();
    let began = tokio::time::Instant::now();
    assert!(orch.start_run(&f.command).await.is_err());
    assert!(began.elapsed() <= Duration::from_secs(6));
    assert_eq!(f.count("scoped-cleanup"), 1);
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
    assert!(matches!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Held(_)
    ));
    let effects = lock(&f.log).len();
    assert!(f.orchestrator().start_run(&f.command).await.is_err());
    assert_eq!(lock(&f.log).len(), effects);
}

#[tokio::test(start_paused = true)]
async fn hanging_postconfirmation_retains_actual_handle_and_all_fences_at_one_deadline() {
    let f = Fixture::new(true);
    f.provider.hang_cleanup.store(true, Ordering::SeqCst);
    let orch = f
        .orchestrator()
        .with_cleanup_timeout(Duration::from_secs(5));
    let began = tokio::time::Instant::now();
    assert!(orch.start_run(&f.command).await.is_err());
    assert!(began.elapsed() <= Duration::from_secs(5));
    assert!(lock(&f.provider.instances)[0].upgrade().is_some());
    assert_eq!(f.runs.inner.run.lock().await.state, RunState::CleaningUp);
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
}

struct Transient {
    fail: Arc<AtomicBool>,
}
#[async_trait]
impl RunRuntimeManager for Transient {
    async fn prepare(&self, _: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        Ok(PreparedRunRuntime::default())
    }
    async fn destroy(&self, _: RunId) -> Result<(), RunRuntimeError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(RunRuntimeError::redacted(
                "controlled transient cleanup failure",
            ))
        } else {
            Ok(())
        }
    }
    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        panic!("strict path cannot sweep unrelated transient resources")
    }
}
#[tokio::test]
async fn transient_failure_after_confirmation_retains_lease_until_exact_recovery() {
    let f = Fixture::new(true);
    let fail = Arc::new(AtomicBool::new(true));
    let orch = f
        .orchestrator()
        .with_runtime_manager(Arc::new(Transient { fail: fail.clone() }));
    assert!(orch.start_run(&f.command).await.is_err());
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
    assert!(matches!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Held(_)
    ));
    fail.store(false, Ordering::SeqCst);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(f.count("release"), 1);
    assert_eq!(f.count("completion"), 1);
}

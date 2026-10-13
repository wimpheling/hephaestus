//! Controlled known-generation cleanup; no native/global/orphan absence proof.
#[path = "generation_drain/instances.rs"]
mod instances;
#[path = "generation_drain/ordered_pending.rs"]
mod ordered_pending;
#[path = "generation_drain/pending.rs"]
mod pending;
use super::support::*;
use crate::provider::{launch::LaunchState, ownership::ProviderOwner};
use std::sync::atomic::AtomicBool;

struct DrainWorker {
    worker: MockWorker,
    hold_kill: AtomicBool,
    fail_reap: AtomicBool,
    kill_entered: Notify,
    release_kill: Notify,
    kills: AtomicUsize,
    reaps: AtomicUsize,
}

impl DrainWorker {
    fn new(hold_kill: bool, fail_reap: bool) -> Arc<Self> {
        Arc::new(Self {
            worker: MockWorker::new(),
            hold_kill: AtomicBool::new(hold_kill),
            fail_reap: AtomicBool::new(fail_reap),
            kill_entered: Notify::new(),
            release_kill: Notify::new(),
            kills: AtomicUsize::new(0),
            reaps: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl WorkerBackend for DrainWorker {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        if matches!(command, WorkerCommand::Destroy) {
            return Err(unavailable_error("controlled destroy", "no reap proof"));
        }
        self.worker.request(command).await
    }
    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
        self.worker.subscribe_events()
    }
    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        self.worker.subscribe_process_exit()
    }
    async fn kill(&self) -> Result<(), VmError> {
        self.kills.fetch_add(1, Ordering::Relaxed);
        if self.hold_kill.swap(false, Ordering::Relaxed) {
            self.kill_entered.notify_one();
            self.release_kill.notified().await;
        }
        // A kill request alone deliberately supplies no exit observation.
        Ok(())
    }
    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        self.reaps.fetch_add(1, Ordering::Relaxed);
        if self.fail_reap.swap(false, Ordering::Relaxed) {
            return Err(unavailable_error(
                "controlled reap",
                "original child remains uncertain",
            ));
        }
        self.worker.kill().await?;
        self.worker.wait_process().await
    }
}

struct Spawner {
    worker: Arc<DrainWorker>,
    hold: bool,
    entered: Notify,
    release: Notify,
    calls: AtomicUsize,
}
impl Spawner {
    fn new(worker: Arc<DrainWorker>, hold: bool) -> Arc<Self> {
        Arc::new(Self {
            worker,
            hold,
            entered: Notify::new(),
            release: Notify::new(),
            calls: AtomicUsize::new(0),
        })
    }
}
#[async_trait]
impl WorkerSpawner for Spawner {
    async fn spawn(
        &self,
        _config: Arc<LibkrunConfig>,
        _spec: crate::validation::PreparedSpec,
        _runtime: &std::path::Path,
        _cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        custody.record_backend(self.worker.clone())?;
        if self.hold {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(self.worker.clone())
    }
}
fn owned(config: &LibkrunConfig, spawner: Arc<dyn WorkerSpawner>) -> LibkrunProvider {
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner).unwrap();
    let owner = ProviderOwner::initialize(config, "drain-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    provider
}

#[tokio::test]
async fn drain_rejects_unowned_wrong_scope_and_empty_reopen_preserves_unknown_children() {
    pending::drain_rejects_unowned_wrong_scope_and_empty_reopen_preserves_unknown_children().await;
}

#[tokio::test]
async fn cancelled_drain_keeps_cleanup_owned_and_refuses_live_pending_provision() {
    pending::cancelled_drain_keeps_cleanup_owned_and_refuses_live_pending_provision().await;
}

#[tokio::test]
async fn pending_reap_failure_preserves_launch_cause_and_original_retry_owner() {
    pending::pending_reap_failure_preserves_launch_cause_and_original_retry_owner().await;
}

#[tokio::test]
async fn handed_off_reap_failure_retains_exact_instance_for_drain_retry() {
    instances::handed_off_reap_failure_retains_exact_instance_for_drain_retry().await;
}

#[tokio::test]
async fn concurrent_drains_clean_one_actual_instance_once_and_preserve_history() {
    instances::concurrent_drains_clean_one_actual_instance_once_and_preserve_history().await;
}

#[tokio::test(start_paused = true)]
async fn drain_waits_for_owning_lifecycle_task_queued_before_operation_mutex() {
    instances::drain_waits_for_owning_lifecycle_task_queued_before_operation_mutex().await;
}

#[tokio::test(start_paused = true)]
async fn drain_deadline_reports_uncertainty_without_canceling_held_instance_cleanup() {
    instances::drain_deadline_reports_uncertainty_without_canceling_held_instance_cleanup().await;
}

struct TwoSpawners(std::sync::Mutex<std::collections::VecDeque<Arc<Spawner>>>);

#[async_trait]
impl WorkerSpawner for TwoSpawners {
    async fn spawn(
        &self,
        config: Arc<LibkrunConfig>,
        spec: crate::validation::PreparedSpec,
        runtime: &std::path::Path,
        cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        let spawner = {
            let mut queue = self.0.lock().unwrap();
            let spawner = queue.pop_front().unwrap();
            drop(queue);
            spawner
        };
        spawner.spawn(config, spec, runtime, cgroup, custody).await
    }
}

async fn assert_pending_failure(later_state: &mut watch::Receiver<LaunchState>) {
    let observed = tokio::time::timeout(
        Duration::from_secs(1),
        later_state.wait_for(|state| matches!(state, LaunchState::Held(_))),
    )
    .await
    .unwrap()
    .unwrap();
    let LaunchState::Held(failure) = &*observed else {
        unreachable!()
    };
    assert!(
        failure
            .cause
            .to_string()
            .contains("generation drain requested")
    );
    assert!(
        failure
            .cleanup
            .to_string()
            .contains("original child remains uncertain")
    );
    drop(observed);
}

#[tokio::test]
async fn later_pending_failure_cannot_be_retried_after_waiting_on_earlier_birth() {
    ordered_pending::later_pending_failure_cannot_be_retried_after_waiting_on_earlier_birth().await;
}

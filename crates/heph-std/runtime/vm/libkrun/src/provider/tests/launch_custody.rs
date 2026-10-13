//! Controlled failures/local Child checks; none proves native VM containment.
#[path = "launch_custody/handoff.rs"]
mod handoff;
#[path = "launch_custody/raw_child.rs"]
mod raw_child;
use super::support::*;
use crate::provider::launch::{LaunchState, PendingLaunch};
use std::sync::atomic::AtomicBool;

struct UncertainWorker {
    worker: MockWorker,
    first_reap: AtomicBool,
    reaps: AtomicUsize,
}

impl UncertainWorker {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            worker: MockWorker::new(),
            first_reap: AtomicBool::new(true),
            reaps: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl WorkerBackend for UncertainWorker {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        if matches!(command, WorkerCommand::Destroy) {
            return Err(unavailable_error(
                "controlled destroy",
                "no protocol reap proof",
            ));
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
        Ok(())
    }
    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        self.reaps.fetch_add(1, Ordering::Relaxed);
        if self.first_reap.swap(false, Ordering::Relaxed) {
            return Err(unavailable_error("controlled reap", "first wait failed"));
        }
        self.worker.kill().await?;
        self.worker.wait_process().await
    }
}

struct PausedLaunch {
    worker: Arc<UncertainWorker>,
    entered: Notify,
    release: Notify,
    reject: bool,
}

#[async_trait]
impl WorkerSpawner for PausedLaunch {
    async fn spawn(
        &self,
        _config: Arc<LibkrunConfig>,
        _spec: crate::validation::PreparedSpec,
        _runtime: &std::path::Path,
        _cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        custody.record_backend(self.worker.clone())?;
        self.entered.notify_one();
        self.release.notified().await;
        if self.reject {
            Err(unavailable_error(
                "worker configuration",
                "controlled configure failure",
            ))
        } else {
            Ok(self.worker.clone())
        }
    }
}

fn spawner(reject: bool) -> Arc<PausedLaunch> {
    Arc::new(PausedLaunch {
        worker: UncertainWorker::new(),
        entered: Notify::new(),
        release: Notify::new(),
        reject,
    })
}

async fn held(entry: &PendingLaunch) {
    let mut state = entry.state.subscribe();
    tokio::time::timeout(
        Duration::from_secs(1),
        state.wait_for(|state| matches!(state, LaunchState::Held(_))),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn cancelled_provision_retains_original_owner_resources_and_id_until_retry() {
    handoff::cancelled_provision_retains_original_owner_resources_and_id_until_retry().await;
}

#[tokio::test]
async fn configure_failure_preserves_cause_and_uncertainty_for_exact_scoped_retry() {
    handoff::configure_failure_preserves_cause_and_uncertainty_for_exact_scoped_retry().await;
}

#[tokio::test]
async fn lost_queued_success_requires_instance_cleanup_before_entry_retirement() {
    handoff::lost_queued_success_requires_instance_cleanup_before_entry_retirement().await;
}

#[tokio::test]
async fn raw_child_survives_placement_failure_without_transport_and_is_actually_reaped() {
    raw_child::raw_child_survives_placement_failure_without_transport_and_is_actually_reaped()
        .await;
}

#[tokio::test]
async fn exclusive_mkdir_failure_never_adopts_or_deletes_foreign_children() {
    raw_child::exclusive_mkdir_failure_never_adopts_or_deletes_foreign_children().await;
}

#[tokio::test]
async fn cancelled_nonconnecting_raw_launch_is_reaped_before_id_and_files_release() {
    raw_child::cancelled_nonconnecting_raw_launch_is_reaped_before_id_and_files_release().await;
}

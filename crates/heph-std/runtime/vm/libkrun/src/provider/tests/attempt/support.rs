//! Controlled backend events/cleanup; no kernel VM or cgroup proof.
use super::super::support::*;
use std::sync::atomic::AtomicBool;

pub(super) struct Spawner {
    pub entered: Notify,
    pub release: Notify,
    pub paused: AtomicBool,
    pub fail: bool,
    pub calls: AtomicUsize,
    pub workers: Mutex<Vec<Arc<dyn WorkerBackend>>>,
}
impl Spawner {
    pub fn new(workers: Vec<Arc<dyn WorkerBackend>>, paused: bool, fail: bool) -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            release: Notify::new(),
            paused: AtomicBool::new(paused),
            fail,
            calls: AtomicUsize::new(0),
            workers: Mutex::new(workers),
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        let worker = self.workers.lock().await.remove(0);
        custody.record_backend(worker.clone())?;
        self.entered.notify_one();
        if self.paused.load(Ordering::SeqCst) {
            self.release.notified().await;
        }
        if self.fail {
            return Err(unavailable_error(
                "controlled spawn",
                "failed with original backend retained",
            ));
        }
        Ok(worker)
    }
}
pub(super) struct FailedKill {
    pub worker: Arc<MockWorker>,
    pub failures: AtomicUsize,
}
#[async_trait]
impl WorkerBackend for FailedKill {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        if matches!(command, WorkerCommand::Destroy) {
            return Err(unavailable_error(
                "controlled destroy",
                "not termination evidence",
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
        if self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(unavailable_error("controlled kill", "still alive"));
        }
        self.worker.kill().await
    }
    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        self.worker.wait_process().await
    }
}
pub(super) async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("bounded controlled attempt")
}
pub(super) fn raw(error: &VmError) -> &VmError {
    let mut current: &dyn std::error::Error = error;
    while let Some(source) = current.source() {
        if let Some(original) = source.downcast_ref::<VmError>() {
            return original;
        }
        current = source;
    }
    panic!("actual original VmError must be retained in source chain")
}

pub(super) struct HeldKill {
    pub worker: Arc<MockWorker>,
    pub entered: Notify,
    pub release: Notify,
}
#[async_trait]
impl WorkerBackend for HeldKill {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        if matches!(command, WorkerCommand::Destroy) {
            return Err(unavailable_error(
                "controlled destroy",
                "not termination evidence",
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
        self.entered.notify_one();
        self.release.notified().await;
        self.worker.kill().await
    }
    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        self.worker.wait_process().await
    }
}

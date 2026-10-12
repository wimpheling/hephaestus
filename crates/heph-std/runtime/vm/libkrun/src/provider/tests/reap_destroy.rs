//! Controlled lifecycle/poll failures exercise custody; none is native stop proof.
use super::support::*;
use crate::provider::worker_client::publish_process_poll;
use std::{collections::VecDeque, sync::atomic::AtomicBool};

#[derive(Clone, Copy)]
enum Failure {
    None,
    Kill,
    Reap,
}

struct ControlledWorker {
    worker: MockWorker,
    fail_kill: AtomicBool,
    fail_reap: AtomicBool,
    kills: AtomicUsize,
    reaps: AtomicUsize,
}

impl ControlledWorker {
    fn new(failure: Failure) -> Arc<Self> {
        Arc::new(Self {
            worker: MockWorker::new(),
            fail_kill: AtomicBool::new(matches!(failure, Failure::Kill)),
            fail_reap: AtomicBool::new(matches!(failure, Failure::Reap)),
            kills: AtomicUsize::new(0),
            reaps: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl WorkerBackend for ControlledWorker {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        if matches!(command, WorkerCommand::Destroy) {
            return Err(unavailable_error(
                "controlled destroy",
                "no protocol exit proof",
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
        self.kills.fetch_add(1, Ordering::Relaxed);
        if self.fail_kill.swap(false, Ordering::Relaxed) {
            return Err(unavailable_error("controlled kill", "termination failed"));
        }
        // A successful kill request still supplies no reaped observation.
        Ok(())
    }

    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        self.reaps.fetch_add(1, Ordering::Relaxed);
        if self.fail_reap.swap(false, Ordering::Relaxed) {
            return Err(unavailable_error("controlled reap", "wait failed"));
        }
        self.worker.kill().await?;
        self.worker.wait_process().await
    }
}

struct QueuedSpawner(Mutex<VecDeque<Arc<dyn WorkerBackend>>>);

#[async_trait]
impl WorkerSpawner for QueuedSpawner {
    async fn spawn(
        &self,
        _config: Arc<LibkrunConfig>,
        _spec: crate::validation::PreparedSpec,
        _runtime: &std::path::Path,
        _cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        let backend = self.0.lock().await.pop_front().unwrap();
        custody.record_backend(Arc::clone(&backend))?;
        Ok(backend)
    }
}

fn provider(config: LibkrunConfig, workers: Vec<Arc<dyn WorkerBackend>>) -> LibkrunProvider {
    LibkrunProvider::new_with_spawner(config, Arc::new(QueuedSpawner(Mutex::new(workers.into()))))
        .unwrap()
}

#[tokio::test]
async fn failed_kill_or_reap_keeps_resources_and_id_until_serialized_retry() {
    for failure in [Failure::Kill, Failure::Reap] {
        let temp = TempDir::new().unwrap();
        let (config, root, runtime, cgroups) = emulated_config(&temp);
        let worker = ControlledWorker::new(failure);
        let provider = provider(config, vec![worker.clone()]);
        let requested = spec("retry-destroy", root);
        let vm = provider.provision(requested.clone()).await.unwrap();
        let id = vm.id().clone();
        assert!(vm.destroy().await.is_err());
        assert!(worker.worker.process_exit.borrow().is_none());
        assert!(runtime.join(&id.0).exists());
        assert!(cgroups.join(&id.0).exists());
        assert!(
            provider
                .ownership_test_inner()
                .ids
                .lock()
                .await
                .contains(&id)
        );
        assert!(matches!(vm.start().await, Err(VmError::Destroyed)));
        assert!(matches!(
            provider.provision(requested).await,
            Err(VmError::AlreadyExists(_))
        ));

        let (first, second) = tokio::join!(vm.destroy(), vm.destroy());
        first.unwrap();
        second.unwrap();
        assert_eq!(worker.kills.load(Ordering::Relaxed), 2);
        assert_eq!(
            worker.reaps.load(Ordering::Relaxed),
            if matches!(failure, Failure::Reap) {
                2
            } else {
                1
            }
        );
        assert!(worker.worker.process_exit.borrow().is_some());
        assert!(!runtime.join(&id.0).exists());
        assert!(!cgroups.join(&id.0).exists());
        assert!(
            !provider
                .ownership_test_inner()
                .ids
                .lock()
                .await
                .contains(&id)
        );
        drop(provider);
    }
}

#[tokio::test]
async fn old_destroyed_handle_cannot_release_or_remove_reused_id() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let old_worker = ControlledWorker::new(Failure::None);
    let new_worker = ControlledWorker::new(Failure::None);
    let provider = provider(config, vec![old_worker.clone(), new_worker.clone()]);
    let requested = spec("reused-id", root);
    let old = provider.provision(requested.clone()).await.unwrap();
    old.destroy().await.unwrap();
    let new = provider.provision(requested).await.unwrap();
    let id = new.id().clone();
    let sentinel = runtime.join(&id.0).join("new-birth");
    fs::write(&sentinel, b"new bytes").unwrap();
    old.destroy().await.unwrap();
    assert_eq!(fs::read(&sentinel).unwrap(), b"new bytes");
    assert!(cgroups.join(&id.0).exists());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert_eq!(old_worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(new_worker.kills.load(Ordering::Relaxed), 0);
    assert!(new_worker.worker.process_exit.borrow().is_none());
    new.destroy().await.unwrap();
    drop(provider);
}

#[test]
fn reap_error_is_not_exit_and_later_confirmed_poll_can_complete() {
    let (exit, _) = watch::channel(None);
    let (errors, _) = watch::channel(None);
    let error = publish_process_poll(
        Err(std::io::Error::other("controlled wait failure")),
        &exit,
        &errors,
    )
    .unwrap_err();
    assert!(error.to_string().contains("controlled wait failure"));
    assert!(
        exit.borrow().is_none(),
        "reap error cannot fabricate a process exit"
    );
    assert!(errors.borrow().is_some());
    assert!(
        publish_process_poll(Ok(None), &exit, &errors)
            .unwrap()
            .is_none()
    );
    assert!(exit.borrow().is_none());
    assert!(errors.borrow().is_none());
    let status = publish_process_poll(Ok(Some(ExitStatus::from_raw(0))), &exit, &errors)
        .unwrap()
        .unwrap();
    assert_eq!(status.code, Some(0));
    assert_eq!(exit.borrow().as_ref().unwrap().code, Some(0));
    assert!(errors.borrow().is_none());
}

#[tokio::test]
async fn cancelled_start_waiter_relinquishes_operation_lock_for_destroy() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::blocking_start());
    let vm = instance(&temp, worker.clone());
    let first = tokio::spawn({
        let vm = vm.clone();
        async move { vm.start().await }
    });
    worker.start_entered.notified().await;
    // Cancellation injects a lost caller, never supplies a physical stop proof.
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(worker.process_exit.borrow().is_none());
    let waiter = tokio::spawn({
        let vm = vm.clone();
        async move { vm.start().await }
    });
    tokio::task::yield_now().await;
    tokio::time::timeout(Duration::from_secs(1), vm.destroy())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(waiter.await.unwrap(), Err(VmError::Destroyed)));
    assert!(worker.process_exit.borrow().is_some());
    drop(vm);
}

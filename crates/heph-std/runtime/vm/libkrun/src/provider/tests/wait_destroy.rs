//! Real subprocess reap with controlled VM protocol/cleanup; no native guest proof.
use super::support::*;
use std::{future::Future, process::Stdio, task::Context};

struct RawWorker {
    protocol: MockWorker,
    process: Arc<WorkerProcess>,
    hold_cleanup: bool,
    reaped: Notify,
    release_cleanup: Notify,
}

#[async_trait]
impl WorkerBackend for RawWorker {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        // Destroy is only a protocol hint. The actual Child must be killed/reaped.
        if matches!(command, WorkerCommand::Destroy) {
            return Ok(());
        }
        self.protocol.request(command).await
    }

    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
        self.protocol.subscribe_events()
    }

    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        self.process.subscribe_process_exit()
    }

    async fn kill(&self) -> Result<(), VmError> {
        self.process.kill().await
    }

    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        let status = self.process.wait_process().await?;
        if self.hold_cleanup {
            self.reaped.notify_one();
            self.release_cleanup.notified().await;
        }
        Ok(status)
    }
}

struct RawSpawner {
    hold_cleanup: bool,
    worker: std::sync::Mutex<Option<Arc<RawWorker>>>,
}

impl RawSpawner {
    fn new(hold_cleanup: bool) -> Arc<Self> {
        Arc::new(Self {
            hold_cleanup,
            worker: std::sync::Mutex::new(None),
        })
    }

    fn worker(&self) -> Arc<RawWorker> {
        self.worker.lock().unwrap().as_ref().unwrap().clone()
    }
}

#[async_trait]
impl WorkerSpawner for RawSpawner {
    async fn spawn(
        &self,
        _config: Arc<LibkrunConfig>,
        _spec: crate::validation::PreparedSpec,
        _runtime: &std::path::Path,
        _cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        let mut command = tokio::process::Command::new("/bin/sleep");
        command
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let process = custody.spawn(&mut command)?;
        let worker = Arc::new(RawWorker {
            protocol: MockWorker::new(),
            process,
            hold_cleanup: self.hold_cleanup,
            reaped: Notify::new(),
            release_cleanup: Notify::new(),
        });
        let backend: Arc<dyn WorkerBackend> = worker.clone();
        custody.record_backend(Arc::clone(&backend))?;
        *self.worker.lock().unwrap() = Some(worker);
        Ok(backend)
    }
}

async fn assert_allocation(
    provider: &LibkrunProvider,
    runtime: &std::path::Path,
    cgroups: &std::path::Path,
    id: &VmId,
    retained: bool,
) {
    assert_eq!(runtime.join(&id.0).exists(), retained);
    assert_eq!(cgroups.join(&id.0).exists(), retained);
    assert_eq!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(id),
        retained
    );
}

#[tokio::test]
async fn never_started_wait_ignores_raw_reap_until_destroy_cleanup_completes() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let spawner = RawSpawner::new(true);
    let provider = LibkrunProvider::new_with_spawner(config, spawner.clone()).unwrap();
    let vm = provider
        .provision(spec("never-started-wait", root))
        .await
        .unwrap();
    let worker = spawner.worker();
    drop(spawner);
    let concrete = provider
        .ownership_test_inner()
        .registry
        .instances()
        .pop()
        .unwrap();
    let id = vm.id().clone();
    let mut events = vm.subscribe_events();
    let mut waiter = Box::pin(vm.wait());
    assert!(
        waiter
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );

    // This existing cleanup duty signals that the actual process monitor has
    // handled the raw exit, rather than relying on scheduler timing or sleep.
    let (sender, handled) = tokio::sync::oneshot::channel();
    concrete.private_http_waiters.lock().await.insert(1, sender);
    let destroying = tokio::spawn({
        let vm = Arc::clone(&vm);
        async move { vm.destroy().await }
    });
    timeout(Duration::from_secs(5), worker.reaped.notified())
        .await
        .unwrap();
    let error = timeout(Duration::from_secs(5), handled)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("guest exited before private HTTP response")
    );
    assert!(worker.process.subscribe_process_exit().borrow().is_some());
    assert!(matches!(
        *concrete.state.lock().await,
        Lifecycle::Destroying { was_started: false }
    ));
    assert!(concrete.terminal.borrow().is_none());
    assert!(matches!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    drop(events);
    assert!(
        waiter
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    assert_allocation(&provider, &runtime, &cgroups, &id, true).await;
    assert!(concrete.resources.lock().await.is_some());

    worker.release_cleanup.notify_one();
    drop(worker);
    timeout(Duration::from_secs(5), destroying)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(waiter.await, Err(VmError::Destroyed)));
    assert!(matches!(vm.wait().await, Err(VmError::Destroyed)));
    drop(vm);
    assert!(matches!(
        *concrete.terminal.borrow(),
        Some(Terminal::Destroyed)
    ));
    assert_allocation(&provider, &runtime, &cgroups, &id, false).await;
    drop(provider);
    assert!(concrete.resources.lock().await.is_none());
    drop(concrete);
}

#[tokio::test]
async fn started_wait_keeps_actual_cached_exit_across_destroy() {
    let temp = TempDir::new().unwrap();
    let (config, root, _, _) = emulated_config(&temp);
    let spawner = RawSpawner::new(false);
    let provider = LibkrunProvider::new_with_spawner(config, spawner.clone()).unwrap();
    let vm = provider
        .provision(spec("started-cached-wait", root))
        .await
        .unwrap();
    vm.start().await.unwrap();
    vm.stop(StopMode::Force).await.unwrap();
    let (first, second) = tokio::join!(vm.wait(), vm.wait());
    let exit = first.unwrap();
    assert_eq!(second.unwrap(), exit);
    let worker = spawner.worker();
    drop(spawner);
    let raw_exit = worker.process.wait_process().await.unwrap().into_vm_exit();
    drop(worker);
    assert_eq!(exit, raw_exit);
    vm.destroy().await.unwrap();
    assert_eq!(vm.wait().await.unwrap(), exit);
    assert_eq!(vm.wait().await.unwrap(), exit);
    drop(vm);
    drop(provider);
}

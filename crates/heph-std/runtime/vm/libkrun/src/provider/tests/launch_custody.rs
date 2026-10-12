//! Controlled failures/local Child checks; none proves native VM containment.
use super::support::*;
use crate::provider::{
    launch::{LaunchState, PendingLaunch, run_launch},
    ownership::ProviderOwner,
};
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
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let spawner = spawner(false);
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let requested = spec("cancelled-launch", root);
    let id = requested.id.clone();
    let task = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(requested).await }
    });
    spawner.entered.notified().await;
    // Abort only the public caller, never the private launch owner or worker.
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let entry = provider
        .ownership_test_inner()
        .pending_launches
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .unwrap();
    held(&entry).await;
    assert!(spawner.worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join(&id.0).is_dir());
    assert!(cgroups.join(&id.0).is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(matches!(
        provider
            .provision(spec(&id.0, temp.path().join("images/root")))
            .await,
        Err(VmError::AlreadyExists(_))
    ));
    drop(provider);
    assert!(
        ProviderOwner::initialize(&config, "test-host").is_err(),
        "uncertain launch must retain original supervisor"
    );
    entry.retry_cleanup().await.unwrap();
    entry.retry_cleanup().await.unwrap();
    assert!(spawner.worker.worker.process_exit.borrow().is_some());
    assert_eq!(spawner.worker.reaps.load(Ordering::Relaxed), 2);
    assert!(!runtime.join(&id.0).exists());
    assert!(!cgroups.join(&id.0).exists());
    let reopened = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(owner) = ProviderOwner::initialize(&config, "test-host") {
                break owner;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(reopened);
    drop(entry);
    drop(spawner);
}

#[tokio::test]
async fn configure_failure_preserves_cause_and_uncertainty_for_exact_scoped_retry() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let spawner = spawner(true);
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let scope = owner.scope().unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let requested = spec("failed-configure", root);
    let id = requested.id.clone();
    let task = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(requested).await }
    });
    spawner.entered.notified().await;
    spawner.release.notify_one();
    let error = task.await.unwrap().err().unwrap();
    assert!(error.to_string().contains("controlled configure failure"));
    assert!(error.to_string().contains("first wait failed"));
    let entry = provider
        .ownership_test_inner()
        .pending_launches
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .unwrap();
    let state = entry.state.borrow().clone();
    let LaunchState::Held(failure) = state else {
        panic!("launch not retained");
    };
    assert!(
        failure
            .cause
            .to_string()
            .contains("controlled configure failure")
    );
    assert!(failure.cleanup.to_string().contains("first wait failed"));
    assert!(runtime.join(&id.0).is_dir());
    assert!(cgroups.join(&id.0).is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    provider.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    assert!(
        provider
            .ownership_test_inner()
            .pending_launches
            .lock()
            .unwrap()
            .get(&id)
            .is_none()
    );
    assert!(
        !provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(!runtime.join(&id.0).exists());
    assert!(!cgroups.join(&id.0).exists());
    drop(failure);
    drop(entry);
    drop(provider);
    drop(spawner);
}

#[tokio::test]
async fn lost_queued_success_requires_instance_cleanup_before_entry_retirement() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let spawner = spawner(false);
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let requested = spec("lost-handoff", root);
    let prepared = prepare_spec(&config, &requested).unwrap();
    let (entry, retries) = PendingLaunch::new(requested.id.clone(), &config, None).unwrap();
    let inner = provider.launch_test_inner();
    inner.ids.lock().await.insert(entry.id.clone());
    inner
        .pending_launches
        .lock()
        .unwrap()
        .insert(entry.id.clone(), entry.clone());
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(run_launch(
        inner.clone(),
        entry.clone(),
        prepared,
        None,
        sender,
        retries,
    ));
    spawner.entered.notified().await;
    let mut state = entry.state.subscribe();
    spawner.release.notify_one();
    state
        .wait_for(|state| matches!(state, LaunchState::Handoff))
        .await
        .unwrap();
    // The result is queued, but no actual receiver acknowledgement occurred.
    drop(receiver);
    held(&entry).await;
    assert!(inner.ids.lock().await.contains(&entry.id));
    assert!(runtime.join(&entry.id.0).is_dir());
    assert!(cgroups.join(&entry.id.0).is_dir());
    assert!(spawner.worker.worker.process_exit.borrow().is_none());
    provider.cleanup_orphan(&entry.id).await.unwrap();
    task.await.unwrap();
    assert!(!inner.ids.lock().await.contains(&entry.id));
    assert!(inner.pending_launches.lock().unwrap().is_empty());
    assert!(!runtime.join(&entry.id.0).exists());
    drop(state);
    drop(inner);
    drop(entry);
    drop(provider);
    drop(spawner);
}

#[tokio::test]
async fn raw_child_survives_placement_failure_without_transport_and_is_actually_reaped() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = temp.path().join("nonconnecting-worker");
    fs::write(&worker, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
    config.worker_binary = worker;
    let requested = spec("raw-placement", root);
    let prepared = prepare_spec(&config, &requested).unwrap();
    fs::create_dir(runtime.join(&requested.id.0)).unwrap();
    fs::create_dir(cgroups.join(&requested.id.0)).unwrap();
    // A directory cannot accept the placement control write.
    fs::create_dir(cgroups.join(&requested.id.0).join("cgroup.procs")).unwrap();
    let (entry, _retries) = PendingLaunch::new(requested.id.clone(), &config, None).unwrap();
    let error = crate::provider::common::WorkerClient::launch(
        Arc::new(config.clone()),
        prepared,
        &runtime.join(&requested.id.0),
        &Cgroup::existing(&config, &requested.id.0),
        &entry.custody,
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("cgroup-place-worker"));
    let process = entry.custody.process().unwrap().unwrap();
    let exit = process.subscribe_process_exit();
    assert!(process.pid.is_some());
    entry
        .custody
        .terminate(Duration::from_secs(1))
        .await
        .unwrap();
    assert!(
        exit.borrow().is_some(),
        "actual Child reap must publish a real exit"
    );
    // This checks local Child kill/wait only. The ordinary filesystem cannot
    // establish real cgroup membership, descendant drain, or VM containment.
    entry
        .custody
        .terminate(Duration::from_secs(1))
        .await
        .unwrap();
    drop(exit);
    drop(process);
    drop(entry);
}

#[tokio::test]
async fn exclusive_mkdir_failure_never_adopts_or_deletes_foreign_children() {
    for foreign_runtime in [false, true] {
        let temp = TempDir::new().unwrap();
        let (config, root, runtime, cgroups) = emulated_config(&temp);
        let spawner = spawner(false);
        let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
        let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
        assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
        let requested = spec("foreign-child", root);
        let parent = if foreign_runtime { &runtime } else { &cgroups };
        let foreign = parent.join(&requested.id.0);
        fs::create_dir(&foreign).unwrap();
        let sentinel = foreign.join("sentinel");
        fs::write(&sentinel, b"foreign bytes").unwrap();
        assert!(provider.provision(requested.clone()).await.is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"foreign bytes");
        assert!(
            !provider
                .ownership_test_inner()
                .ids
                .lock()
                .await
                .contains(&requested.id)
        );
        assert!(
            provider
                .ownership_test_inner()
                .pending_launches
                .lock()
                .unwrap()
                .is_empty()
        );
        assert_eq!(spawner.worker.reaps.load(Ordering::Relaxed), 0);
        if !foreign_runtime {
            assert!(!runtime.join(&requested.id.0).exists());
        }
        drop(provider);
        drop(spawner);
    }
}

#[tokio::test]
async fn cancelled_nonconnecting_raw_launch_is_reaped_before_id_and_files_release() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    let binary = temp.path().join("nonconnecting-worker");
    fs::write(&binary, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    config.worker_binary = binary;
    let provider = LibkrunProvider::new(config).unwrap();
    let requested = spec("cancel-raw-ipc", root);
    let id = requested.id.clone();
    let task = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(requested).await }
    });
    let (entry, process) = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            {
                let entry = provider
                    .ownership_test_inner()
                    .pending_launches
                    .lock()
                    .unwrap()
                    .get(&id)
                    .cloned();
                if let Some(entry) = entry {
                    if let Some(process) = entry.custody.process().unwrap() {
                        break (entry, process);
                    }
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let exit = process.subscribe_process_exit();
    assert!(exit.borrow().is_none());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let mut state = entry.state.subscribe();
    tokio::time::timeout(
        Duration::from_secs(1),
        state.wait_for(|state| matches!(state, LaunchState::Cleaned)),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        exit.borrow().is_some(),
        "actual raw Child reap precedes cleanup completion"
    );
    assert!(
        !provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(!runtime.join(&id.0).exists());
    assert!(!cgroups.join(&id.0).exists());
    // Emulated cgroup files cannot prove native containment or descendant drain.
    drop(state);
    drop(exit);
    drop(process);
    drop(entry);
    drop(provider);
}

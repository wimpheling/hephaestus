//! Controlled known-generation cleanup; no native/global/orphan absence proof.
use super::support::*;
use crate::provider::{VmAttemptPhase, launch::LaunchState, ownership::ProviderOwner};
use std::{future::Future, sync::atomic::AtomicBool, task::Context};

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
    let temp = TempDir::new().unwrap();
    let (config, _, runtime, cgroups) = emulated_config(&temp);
    let worker = DrainWorker::new(false, false);
    let unowned =
        LibkrunProvider::new_with_spawner(config.clone(), Spawner::new(worker.clone(), false))
            .unwrap();
    let provider = owned(&config, Spawner::new(worker.clone(), false));
    let scope = provider.owner_scope().unwrap();
    assert!(matches!(
        unowned.drain_generation(&scope).await,
        Err(VmError::Unsupported { .. })
    ));
    let other = TempDir::new().unwrap();
    let (other_config, _, _, _) = emulated_config(&other);
    let other_provider = owned(&other_config, Spawner::new(worker.clone(), false));
    assert!(
        provider
            .drain_generation(&other_provider.owner_scope().unwrap())
            .await
            .is_err()
    );
    assert!(
        !provider
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .admission_closed
    );
    drop(provider);
    fs::create_dir(runtime.join("prior-process-unknown")).unwrap();
    fs::create_dir(cgroups.join("prior-process-unknown")).unwrap();
    let sentinel = runtime.join("prior-process-unknown/sentinel");
    fs::write(&sentinel, b"original unclassified bytes").unwrap();
    let reopened = LibkrunProvider::open_owned(config.clone(), &scope).unwrap();
    reopened.drain_generation(&scope).await.unwrap();
    let inventory = reopened.attempt_inventory(&scope).await.unwrap();
    assert!(inventory.admission_closed);
    assert!(inventory.attempts.is_empty());
    assert_eq!(fs::read(sentinel).unwrap(), b"original unclassified bytes");
    assert!(cgroups.join("prior-process-unknown").is_dir());
    assert_eq!(worker.kills.load(Ordering::Relaxed), 0);
    drop(reopened);
    drop(unowned);
    drop(other_provider);
    drop(worker);
}

#[tokio::test]
async fn cancelled_drain_keeps_cleanup_owned_and_refuses_live_pending_provision() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    config.startup_timeout = Duration::from_secs(5);
    let worker = DrainWorker::new(true, false);
    let spawner = Spawner::new(worker.clone(), true);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let provisioning = tokio::spawn({
        let provider = provider.clone();
        let requested = spec("pending", root.clone());
        async move { provider.provision(requested).await }
    });
    spawner.entered.notified().await;
    let draining = tokio::spawn({
        let provider = provider.clone();
        let scope = scope.clone();
        async move { provider.drain_generation(&scope).await }
    });
    worker.kill_entered.notified().await;
    draining.abort();
    assert!(draining.await.unwrap_err().is_cancelled());
    assert!(!provisioning.is_finished());
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("pending").is_dir());
    assert!(cgroups.join("pending").is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&VmId("pending".into()))
    );
    assert!(provider.provision(spec("late", root)).await.is_err());
    assert_eq!(spawner.calls.load(Ordering::Relaxed), 1);
    assert!(LibkrunProvider::open_owned(config.clone(), &scope).is_err());
    worker.release_kill.notify_one();
    let error = provisioning.await.unwrap().err().unwrap();
    assert!(error.to_string().contains("generation drain requested"));
    // A second owning drain joins/serializes behind the canceled caller's task.
    provider.drain_generation(&scope).await.unwrap();
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::Cleaned);
    assert_eq!(inventory.attempts[0].operations, 0);
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    assert!(!runtime.join("pending").exists());
    assert!(!cgroups.join("pending").exists());
    assert!(provider.ownership_test_inner().ids.lock().await.is_empty());
    drop(provider);
    drop(spawner);
    drop(worker);
}

#[tokio::test]
async fn pending_reap_failure_preserves_launch_cause_and_original_retry_owner() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = DrainWorker::new(false, true);
    let spawner = Spawner::new(worker.clone(), true);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let provisioning = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(spec("held-pending", root)).await }
    });
    spawner.entered.notified().await;
    let error = provider.drain_generation(&scope).await.unwrap_err();
    assert!(error.to_string().contains("generation drain requested"));
    assert!(
        error
            .to_string()
            .contains("original child remains uncertain")
    );
    let launch_error = provisioning.await.unwrap().err().unwrap();
    assert!(
        launch_error
            .to_string()
            .contains("generation drain requested")
    );
    assert!(
        launch_error
            .to_string()
            .contains("original child remains uncertain")
    );
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("held-pending").is_dir());
    assert!(cgroups.join("held-pending").is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&VmId("held-pending".into()))
    );
    assert!(LibkrunProvider::open_owned(config.clone(), &scope).is_err());
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    provider.drain_generation(&scope).await.unwrap();
    provider.drain_generation(&scope).await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 2);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 2);
    assert!(worker.worker.process_exit.borrow().is_some());
    assert!(!runtime.join("held-pending").exists());
    assert!(!cgroups.join("held-pending").exists());
    assert!(
        provider
            .ownership_test_inner()
            .pending_launches
            .lock()
            .unwrap()
            .is_empty()
    );
    drop(provider);
    drop(spawner);
    drop(worker);
}

#[tokio::test]
async fn handed_off_reap_failure_retains_exact_instance_for_drain_retry() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = DrainWorker::new(false, true);
    let spawner = Spawner::new(worker.clone(), false);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("instance", root)).await.unwrap();
    let error = provider.drain_generation(&scope).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("original child remains uncertain")
    );
    assert!(runtime.join("instance").is_dir());
    assert!(cgroups.join("instance").is_dir());
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(vm.start().await.is_err());
    assert_eq!(worker.worker.start_calls.load(Ordering::Relaxed), 0);
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(vm.id())
    );
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::CleanupPending);
    assert!(
        inventory.attempts[0]
            .error
            .as_ref()
            .unwrap()
            .contains("original child remains uncertain")
    );
    provider.drain_generation(&scope).await.unwrap();
    vm.destroy().await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 2);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 2);
    assert_eq!(spawner.calls.load(Ordering::Relaxed), 1);
    assert!(!runtime.join("instance").exists());
    assert!(!cgroups.join("instance").exists());
    drop(vm);
    drop(provider);
    drop(spawner);
    drop(worker);
}

#[tokio::test]
async fn concurrent_drains_clean_one_actual_instance_once_and_preserve_history() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = DrainWorker::new(false, false);
    let spawner = Spawner::new(worker.clone(), false);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("concurrent", root)).await.unwrap();
    vm.start().await.unwrap();
    let (first, second) = tokio::join!(
        provider.drain_generation(&scope),
        provider.drain_generation(&scope)
    );
    first.unwrap();
    second.unwrap();
    provider.drain_generation(&scope).await.unwrap();
    vm.destroy().await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts.len(), 1);
    assert_eq!(inventory.attempts[0].birth, 1);
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::Cleaned);
    assert_eq!(inventory.attempts[0].operations, 0);
    assert!(provider.ownership_test_inner().ids.lock().await.is_empty());
    assert!(!runtime.join("concurrent").exists());
    assert!(!cgroups.join("concurrent").exists());
    drop(vm);
    drop(provider);
    drop(spawner);
    drop(worker);
}

#[tokio::test(start_paused = true)]
async fn drain_waits_for_owning_lifecycle_task_queued_before_operation_mutex() {
    let temp = TempDir::new().unwrap();
    let (config, root, _, _) = emulated_config(&temp);
    let worker = DrainWorker::new(false, false);
    let provider = owned(&config, Spawner::new(worker.clone(), false));
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("queued", root)).await.unwrap();
    let instance = provider
        .ownership_test_inner()
        .registry
        .instances()
        .pop()
        .unwrap();
    vm.destroy().await.unwrap();
    provider
        .ownership_test_inner()
        .registry
        .wait_settled()
        .await;
    let operation = instance.operation.lock().await;
    let mut queued = Box::pin(vm.start());
    // Poll the public future once: registration/spawn are synchronous, and its
    // owning task must now queue behind the actual instance's operation mutex.
    assert!(
        queued
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    let mut draining = Box::pin(provider.drain_generation(&scope));
    // This negative deadline drops only the borrowed waiter, preserving both
    // owning tasks. Without queued-task bookkeeping drain would return early.
    assert!(
        tokio::time::timeout(Duration::from_millis(1), &mut draining)
            .await
            .is_err()
    );
    drop(operation);
    assert!(matches!(
        queued.await,
        Err(VmError::InvalidState("VM provider admission is closed"))
    ));
    draining.await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    drop(instance);
    drop(vm);
    drop(provider);
    drop(worker);
}

#[tokio::test(start_paused = true)]
async fn drain_deadline_reports_uncertainty_without_canceling_held_instance_cleanup() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    config.startup_timeout = Duration::from_millis(10);
    let worker = DrainWorker::new(true, false);
    let provider = owned(&config, Spawner::new(worker.clone(), false));
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("deadline", root)).await.unwrap();
    let draining = tokio::spawn({
        let provider = provider.clone();
        let scope = scope.clone();
        async move { provider.drain_generation(&scope).await }
    });
    worker.kill_entered.notified().await;
    tokio::time::advance(Duration::from_millis(10)).await;
    let error = draining.await.unwrap().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("instance cleanup remains unconfirmed")
    );
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("deadline").is_dir());
    assert!(cgroups.join("deadline").is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(vm.id())
    );
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::CleanupPending);
    assert_eq!(inventory.attempts[0].operations, 1);
    worker.release_kill.notify_one();
    provider.drain_generation(&scope).await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    assert!(!runtime.join("deadline").exists());
    assert!(!cgroups.join("deadline").exists());
    drop(vm);
    drop(provider);
    drop(worker);
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
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let earlier_worker = DrainWorker::new(true, false);
    let later_worker = DrainWorker::new(false, true);
    let earlier = Spawner::new(earlier_worker.clone(), true);
    let later = Spawner::new(later_worker.clone(), true);
    let provider = owned(
        &config,
        Arc::new(TwoSpawners(std::sync::Mutex::new(
            std::collections::VecDeque::from([earlier.clone(), later.clone()]),
        ))),
    );
    let scope = provider.owner_scope().unwrap();
    let first_provision = tokio::spawn({
        let provider = provider.clone();
        let requested = spec("earlier", root.clone());
        async move { provider.provision(requested).await }
    });
    earlier.entered.notified().await;
    let second_provision = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(spec("later", root)).await }
    });
    later.entered.notified().await;
    let later_entry = provider
        .ownership_test_inner()
        .pending_launches
        .lock()
        .unwrap()
        .get(&VmId("later".into()))
        .cloned()
        .unwrap();
    let mut later_state = later_entry.state.subscribe();
    let draining = tokio::spawn({
        let provider = provider.clone();
        let scope = scope.clone();
        async move { provider.drain_generation(&scope).await }
    });
    earlier_worker.kill_entered.notified().await;
    // Observe the real later owner's first failure while the earlier kill still
    // blocks the sequential drain pass. No delay or fabricated Held state.
    assert_pending_failure(&mut later_state).await;
    assert!(!draining.is_finished());
    assert_eq!(later_worker.reaps.load(Ordering::Relaxed), 1);
    earlier_worker.release_kill.notify_one();
    let error = draining.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("generation drain requested"));
    assert!(
        error
            .to_string()
            .contains("original child remains uncertain")
    );
    assert!(first_provision.await.unwrap().is_err());
    let refused = second_provision.await.unwrap().err().unwrap();
    assert!(
        refused
            .to_string()
            .contains("original child remains uncertain")
    );
    assert!(matches!(*later_entry.state.borrow(), LaunchState::Held(_)));
    assert_eq!(later_worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(later_worker.reaps.load(Ordering::Relaxed), 1);
    assert!(later_worker.worker.process_exit.borrow().is_none());
    for allocation in [&runtime, &cgroups] {
        assert!(!allocation.join("earlier").exists());
        assert!(allocation.join("later").is_dir());
    }
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&VmId("later".into()))
    );
    assert!(Arc::ptr_eq(
        &later_entry,
        provider
            .ownership_test_inner()
            .pending_launches
            .lock()
            .unwrap()
            .get(&VmId("later".into()))
            .unwrap()
    ));
    provider.drain_generation(&scope).await.unwrap();
    assert_eq!(later_worker.kills.load(Ordering::Relaxed), 2);
    assert_eq!(later_worker.reaps.load(Ordering::Relaxed), 2);
    assert!(later_worker.worker.process_exit.borrow().is_some());
    assert_eq!(earlier_worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(earlier_worker.reaps.load(Ordering::Relaxed), 1);
    assert_eq!(earlier.calls.load(Ordering::Relaxed), 1);
    assert_eq!(later.calls.load(Ordering::Relaxed), 1);
    assert!(provider.ownership_test_inner().ids.lock().await.is_empty());
    assert!(!runtime.join("later").exists());
    assert!(!cgroups.join("later").exists());
    drop(later_entry);
    drop(provider);
    drop(earlier);
    drop(later);
    drop(earlier_worker);
    drop(later_worker);
}

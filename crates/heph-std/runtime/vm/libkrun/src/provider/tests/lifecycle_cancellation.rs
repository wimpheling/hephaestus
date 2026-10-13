//! Controlled cancellation of lifecycle callers; no native process/drain proof.
use super::support::*;
use crate::provider::{VmAttemptPhase, VmAttemptSnapshot, ownership::ProviderOwner};

#[derive(Clone, Copy)]
enum Script {
    Normal,
    HeldKill,
    HeldReapError,
    KillError,
}

struct HeldWorker {
    worker: MockWorker,
    script: std::sync::Mutex<Script>,
    kill_entered: Notify,
    release_kill: Notify,
    reap_entered: Notify,
    release_reap: Notify,
    kills: AtomicUsize,
    reaps: AtomicUsize,
}

impl HeldWorker {
    fn new(worker: MockWorker, script: Script) -> Arc<Self> {
        Arc::new(Self {
            worker,
            script: std::sync::Mutex::new(script),
            kill_entered: Notify::new(),
            release_kill: Notify::new(),
            reap_entered: Notify::new(),
            release_reap: Notify::new(),
            kills: AtomicUsize::new(0),
            reaps: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl WorkerBackend for HeldWorker {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        if matches!(command, WorkerCommand::Destroy) {
            return Err(unavailable_error("controlled destroy", "no process proof"));
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
        let script = {
            let mut script = self.script.lock().unwrap();
            let current = *script;
            if matches!(current, Script::HeldKill | Script::KillError) {
                *script = Script::Normal;
            }
            current
        };
        match script {
            Script::HeldKill => {
                self.kill_entered.notify_one();
                self.release_kill.notified().await;
            }
            Script::KillError => {
                return Err(unavailable_error(
                    "controlled kill",
                    "held termination failure",
                ));
            }
            Script::HeldReapError => return Ok(()),
            Script::Normal => {}
        }
        self.worker.kill().await
    }

    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        self.reaps.fetch_add(1, Ordering::Relaxed);
        let held = {
            let mut script = self.script.lock().unwrap();
            let held = matches!(*script, Script::HeldReapError);
            if held {
                *script = Script::Normal;
            }
            held
        };
        if held {
            self.reap_entered.notify_one();
            self.release_reap.notified().await;
            return Err(unavailable_error("controlled reap", "held original child"));
        }
        self.worker.wait_process().await
    }
}

struct Spawner(Arc<HeldWorker>);

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
        let worker: Arc<dyn WorkerBackend> = self.0.clone();
        custody.record_backend(Arc::clone(&worker))?;
        Ok(worker)
    }
}

fn owned(config: &LibkrunConfig, worker: Arc<HeldWorker>) -> LibkrunProvider {
    let provider =
        LibkrunProvider::new_with_spawner(config.clone(), Arc::new(Spawner(worker))).unwrap();
    let owner = ProviderOwner::initialize(config, "lifecycle-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    provider
}

async fn finished(
    provider: &LibkrunProvider,
    scope: &vm_trait::VmProviderOwnerScope,
    phase: VmAttemptPhase,
) -> VmAttemptSnapshot {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let inventory = provider.attempt_inventory(scope).await.unwrap();
            let record = &inventory.attempts[0];
            if record.phase == phase && record.operations == 0 {
                return record.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the actual owning operation must finish, not merely lose its caller")
}

#[tokio::test]
async fn aborted_start_caller_does_not_cancel_admitted_start_after_close() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    config.startup_timeout = Duration::from_secs(5);
    let worker = HeldWorker::new(MockWorker::blocking_start(), Script::Normal);
    let provider = owned(&config, worker.clone());
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("owned-start", root)).await.unwrap();
    let starting = tokio::spawn({
        let vm = vm.clone();
        async move { vm.start().await }
    });
    worker.worker.start_entered.notified().await;
    starting.abort();
    assert!(starting.await.unwrap_err().is_cancelled());
    provider.close_admission(&scope).await.unwrap();
    let late = tokio::spawn({
        let vm = vm.clone();
        async move { vm.start().await }
    });
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::Starting);
    assert_eq!(inventory.attempts[0].operations, 1);
    assert!(!late.is_finished());
    assert_eq!(worker.worker.start_calls.load(Ordering::Relaxed), 1);
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("owned-start").is_dir());
    assert!(cgroups.join("owned-start").is_dir());
    worker.worker.release_start.notify_one();
    finished(&provider, &scope, VmAttemptPhase::Running).await;
    assert!(matches!(
        late.await.unwrap(),
        Err(VmError::InvalidState("VM provider admission is closed"))
    ));
    assert_eq!(worker.worker.start_calls.load(Ordering::Relaxed), 1);
    vm.destroy().await.unwrap();
    finished(&provider, &scope, VmAttemptPhase::Cleaned).await;
    drop(vm);
    drop(provider);
    drop(worker);
}

#[tokio::test]
async fn aborted_force_stop_caller_keeps_actual_kill_owned_until_terminal() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = HeldWorker::new(MockWorker::new(), Script::HeldKill);
    let provider = owned(&config, worker.clone());
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("owned-stop", root)).await.unwrap();
    vm.start().await.unwrap();
    let stopping = tokio::spawn({
        let vm = vm.clone();
        async move { vm.stop(StopMode::Force).await }
    });
    worker.kill_entered.notified().await;
    stopping.abort();
    assert!(stopping.await.unwrap_err().is_cancelled());
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::Stopping);
    assert_eq!(inventory.attempts[0].operations, 1);
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("owned-stop").is_dir());
    assert!(cgroups.join("owned-stop").is_dir());
    worker.release_kill.notify_one();
    finished(&provider, &scope, VmAttemptPhase::ExitObserved).await;
    assert_eq!(vm.wait().await.unwrap().signal, Some(9));
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    // Stop observes guest exit; it has not released allocation custody.
    assert!(runtime.join("owned-stop").is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(vm.id())
    );
    vm.destroy().await.unwrap();
    finished(&provider, &scope, VmAttemptPhase::Cleaned).await;
    drop(vm);
    drop(provider);
    drop(worker);
}

#[tokio::test]
async fn aborted_destroy_reap_error_retains_original_owner_and_retry_custody() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    config.startup_timeout = Duration::from_secs(5);
    let worker = HeldWorker::new(MockWorker::new(), Script::HeldReapError);
    let provider = owned(&config, worker.clone());
    let scope = provider.owner_scope().unwrap();
    let requested = spec("owned-destroy", root);
    let vm = provider.provision(requested.clone()).await.unwrap();
    let weak = Arc::downgrade(&vm);
    let destroying = tokio::spawn({
        let vm = vm.clone();
        async move { vm.destroy().await }
    });
    worker.reap_entered.notified().await;
    destroying.abort();
    assert!(destroying.await.unwrap_err().is_cancelled());
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("owned-destroy").is_dir());
    assert!(cgroups.join("owned-destroy").is_dir());
    assert!(matches!(
        provider.provision(requested).await,
        Err(VmError::AlreadyExists(_))
    ));
    worker.release_reap.notify_one();
    let record = finished(&provider, &scope, VmAttemptPhase::CleanupPending).await;
    assert!(record.error.unwrap().contains("held original child"));
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    assert!(worker.worker.process_exit.borrow().is_none());
    drop(vm);
    drop(provider);
    assert!(LibkrunProvider::open_owned(config.clone(), &scope).is_err());
    assert!(runtime.join("owned-destroy").is_dir());
    assert!(cgroups.join("owned-destroy").is_dir());
    let original = weak
        .upgrade()
        .expect("the exact uncertain instance remains owned");
    original.destroy().await.unwrap();
    original.destroy().await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 2);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 2);
    assert!(worker.worker.process_exit.borrow().is_some());
    assert!(!runtime.join("owned-destroy").exists());
    assert!(!cgroups.join("owned-destroy").exists());
    drop(original);
    let reopened = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(provider) = LibkrunProvider::open_owned(config.clone(), &scope) {
                return provider;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        reopened
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .attempts
            .is_empty()
    );
    drop(reopened);
    drop(worker);
}

#[tokio::test]
async fn force_stop_retries_original_backend_after_failed_kill() {
    let temp = TempDir::new().unwrap();
    let (config, root, _, _) = emulated_config(&temp);
    let worker = HeldWorker::new(MockWorker::new(), Script::KillError);
    let provider = owned(&config, worker.clone());
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("retry-stop", root)).await.unwrap();
    vm.start().await.unwrap();
    let error = vm.stop(StopMode::Force).await.unwrap_err();
    assert!(error.to_string().contains("held termination failure"));
    let record = finished(&provider, &scope, VmAttemptPhase::Stopping).await;
    assert!(record.error.unwrap().contains("held termination failure"));
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(vm.id())
    );
    tokio::time::timeout(Duration::from_secs(1), vm.stop(StopMode::Force))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 2);
    assert_eq!(vm.wait().await.unwrap().signal, Some(9));
    vm.destroy().await.unwrap();
    drop(vm);
    drop(provider);
    drop(worker);
}

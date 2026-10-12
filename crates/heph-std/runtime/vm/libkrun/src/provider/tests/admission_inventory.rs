//! Controlled generation races and custody; no native drain claim.
use super::support::*;
use crate::provider::{VmAttemptPhase, ownership::ProviderOwner};
use std::{collections::VecDeque, sync::atomic::AtomicBool};

struct SequenceSpawner {
    workers: std::sync::Mutex<VecDeque<Arc<dyn WorkerBackend>>>,
    calls: AtomicUsize,
}
impl SequenceSpawner {
    fn new(workers: Vec<Arc<dyn WorkerBackend>>) -> Arc<Self> {
        Arc::new(Self {
            workers: std::sync::Mutex::new(workers.into()),
            calls: AtomicUsize::new(0),
        })
    }
}
#[async_trait]
impl WorkerSpawner for SequenceSpawner {
    async fn spawn(
        &self,
        _config: Arc<LibkrunConfig>,
        _spec: crate::validation::PreparedSpec,
        _runtime: &std::path::Path,
        _cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let worker = self.workers.lock().unwrap().pop_front().unwrap();
        custody.record_backend(Arc::clone(&worker))?;
        Ok(worker)
    }
}

fn owned(config: &LibkrunConfig, spawner: Arc<SequenceSpawner>) -> LibkrunProvider {
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner).unwrap();
    let owner = ProviderOwner::initialize(config, "inventory-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    provider
}

#[tokio::test]
async fn close_requires_exact_owned_scope_and_empty_reopen_inventory_is_only_diagnostic() {
    let temp = TempDir::new().unwrap();
    let (config, _, _, _) = emulated_config(&temp);
    let unowned =
        LibkrunProvider::new_with_spawner(config.clone(), SequenceSpawner::new(vec![])).unwrap();
    let provider = owned(&config, SequenceSpawner::new(vec![]));
    let scope = provider.owner_scope().unwrap();
    assert!(matches!(
        unowned.close_admission(&scope).await,
        Err(VmError::Unsupported { .. })
    ));
    assert!(matches!(
        unowned.attempt_inventory(&scope).await,
        Err(VmError::Unsupported { .. })
    ));
    let other = TempDir::new().unwrap();
    let (other_config, _, _, _) = emulated_config(&other);
    let other_provider = owned(&other_config, SequenceSpawner::new(vec![]));
    let wrong = other_provider.owner_scope().unwrap();
    assert!(provider.close_admission(&wrong).await.is_err());
    assert!(provider.attempt_inventory(&wrong).await.is_err());
    assert!(
        !provider
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .admission_closed
    );
    provider.close_admission(&scope).await.unwrap();
    provider.close_admission(&scope).await.unwrap();
    assert!(
        provider
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .admission_closed
    );
    drop(provider);
    let unknown_runtime = config.runtime_root.join("prior-process-orphan");
    let unknown_cgroup = config.cgroup_root.join("prior-process-orphan");
    fs::create_dir(&unknown_runtime).unwrap();
    fs::create_dir(&unknown_cgroup).unwrap();
    fs::write(
        unknown_runtime.join("sentinel"),
        b"unclassified original bytes",
    )
    .unwrap();
    assert!(
        LibkrunProvider::new_with_spawner(config.clone(), SequenceSpawner::new(vec![])).is_err(),
        "managed roots must reject the unowned constructor"
    );
    let reopened = LibkrunProvider::open_owned(config.clone(), &scope).unwrap();
    assert_eq!(reopened.owner_scope().unwrap(), scope);
    let inventory = reopened.attempt_inventory(&scope).await.unwrap();
    assert!(inventory.attempts.is_empty());
    assert!(!inventory.admission_closed);
    assert_eq!(
        fs::read(unknown_runtime.join("sentinel")).unwrap(),
        b"unclassified original bytes"
    );
    assert!(unknown_cgroup.is_dir());
    // The new generation reports no admitted births; it has performed no
    // process/placement/orphan inspection and offers no physical receipt.
    drop(reopened);
    drop(unowned);
    drop(other_provider);
}

#[tokio::test]
async fn racing_close_and_two_provisions_inventories_exactly_the_admitted_births() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let spawner = SequenceSpawner::new(vec![
        Arc::new(MockWorker::new()),
        Arc::new(MockWorker::new()),
    ]);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(4));
    let mut tasks = Vec::new();
    for id in ["race-a", "race-b"] {
        let provider = provider.clone();
        let barrier = barrier.clone();
        let requested = spec(id, root.clone());
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            provider.provision(requested).await
        }));
    }
    let closing = tokio::spawn({
        let provider = provider.clone();
        let scope = scope.clone();
        let barrier = barrier.clone();
        async move {
            barrier.wait().await;
            provider.close_admission(&scope).await
        }
    });
    barrier.wait().await;
    closing.await.unwrap().unwrap();
    let mut instances = Vec::new();
    for task in tasks {
        match task.await.unwrap() {
            Ok(instance) => instances.push(instance),
            Err(error) => assert!(matches!(
                error,
                VmError::InvalidState("VM provider admission is closed")
            )),
        }
    }
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert!(inventory.admission_closed);
    assert_eq!(inventory.attempts.len(), instances.len());
    assert_eq!(spawner.calls.load(Ordering::Relaxed), instances.len());
    for id in ["race-a", "race-b"] {
        let admitted = inventory.attempts.iter().any(|attempt| attempt.id.0 == id);
        assert_eq!(runtime.join(id).exists(), admitted);
        assert_eq!(cgroups.join(id).exists(), admitted);
    }
    assert!(provider.provision(spec("late", root)).await.is_err());
    assert!(!runtime.join("late").exists());
    assert!(!cgroups.join("late").exists());
    for instance in instances {
        instance.destroy().await.unwrap();
    }
    assert!(
        provider
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .attempts
            .iter()
            .all(|attempt| attempt.phase == VmAttemptPhase::Cleaned)
    );
    drop(provider);
    drop(spawner);
    drop(barrier);
}

#[tokio::test]
async fn preclose_blocked_start_finishes_but_late_start_has_no_worker_effects() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, _, _) = emulated_config(&temp);
    config.startup_timeout = Duration::from_secs(5);
    config.readiness_timeout = Duration::from_secs(5);
    let early_worker = Arc::new(MockWorker::blocking_start());
    let late_worker = Arc::new(MockWorker::new());
    let provider = owned(
        &config,
        SequenceSpawner::new(vec![early_worker.clone(), late_worker.clone()]),
    );
    let scope = provider.owner_scope().unwrap();
    let early = provider
        .provision(spec("early", root.clone()))
        .await
        .unwrap();
    let late = provider.provision(spec("late", root)).await.unwrap();
    let starting = tokio::spawn({
        let early = early.clone();
        async move { early.start().await }
    });
    early_worker.start_entered.notified().await;
    provider.close_admission(&scope).await.unwrap();
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::Starting);
    assert_eq!(inventory.attempts[0].operations, 1);
    assert!(late.start().await.is_err());
    assert_eq!(late_worker.start_calls.load(Ordering::Relaxed), 0);
    early_worker.release_start.notify_one();
    starting.await.unwrap().unwrap();
    assert_eq!(early_worker.start_calls.load(Ordering::Relaxed), 1);
    early.destroy().await.unwrap();
    late.destroy().await.unwrap();
    drop(early);
    drop(late);
    drop(provider);
    drop(early_worker);
    drop(late_worker);
}

struct FailedReap {
    worker: MockWorker,
    fail: AtomicBool,
}
#[async_trait]
impl WorkerBackend for FailedReap {
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
        Ok(())
    }
    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        if self.fail.swap(false, Ordering::Relaxed) {
            return Err(unavailable_error(
                "controlled reap",
                "retained uncertain child",
            ));
        }
        self.worker.kill().await?;
        self.worker.wait_process().await
    }
}

#[tokio::test]
async fn last_facade_and_instance_drop_preserves_actual_uncertain_instance_and_supervisor() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = Arc::new(FailedReap {
        worker: MockWorker::new(),
        fail: AtomicBool::new(true),
    });
    let provider = owned(&config, SequenceSpawner::new(vec![worker.clone()]));
    let scope = provider.owner_scope().unwrap();
    let instance = provider.provision(spec("retained", root)).await.unwrap();
    assert!(instance.destroy().await.is_err());
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::CleanupPending);
    assert!(
        inventory.attempts[0]
            .error
            .as_ref()
            .unwrap()
            .contains("retained uncertain child")
    );
    let weak = Arc::downgrade(&instance);
    drop(instance);
    drop(provider);
    assert!(ProviderOwner::initialize(&config, "inventory-host").is_err());
    assert!(runtime.join("retained").is_dir());
    assert!(cgroups.join("retained").is_dir());
    let original = weak
        .upgrade()
        .expect("generation keeper must retain the same instance");
    original.destroy().await.unwrap();
    original.destroy().await.unwrap();
    assert!(worker.worker.process_exit.borrow().is_some());
    assert!(!runtime.join("retained").exists());
    assert!(!cgroups.join("retained").exists());
    drop(original);
    let reopened = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(owner) = ProviderOwner::initialize(&config, "inventory-host") {
                break owner;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(reopened);
    drop(worker);
}

#[tokio::test]
async fn reused_id_keeps_distinct_cleaned_births_and_old_destroy_cannot_retire_new_birth() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, _) = emulated_config(&temp);
    let provider = owned(
        &config,
        SequenceSpawner::new(vec![
            Arc::new(MockWorker::new()),
            Arc::new(MockWorker::new()),
        ]),
    );
    let scope = provider.owner_scope().unwrap();
    let old = provider
        .provision(spec("reused", root.clone()))
        .await
        .unwrap();
    old.destroy().await.unwrap();
    let new = provider.provision(spec("reused", root)).await.unwrap();
    old.destroy().await.unwrap();
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts.len(), 2);
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::Cleaned);
    // Acknowledgement has been sent; the private launch task may still be
    // retiring its handoff entry while this diagnostic snapshot is collected.
    assert!(matches!(
        inventory.attempts[1].phase,
        VmAttemptPhase::Provisioned | VmAttemptPhase::Handoff
    ));
    assert_eq!(inventory.attempts[0].id, inventory.attempts[1].id);
    assert_ne!(inventory.attempts[0].birth, inventory.attempts[1].birth);
    assert!(runtime.join("reused").is_dir());
    new.destroy().await.unwrap();
    assert!(
        provider
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .attempts
            .iter()
            .all(|attempt| attempt.phase == VmAttemptPhase::Cleaned)
    );
    drop(old);
    drop(new);
    drop(provider);
}

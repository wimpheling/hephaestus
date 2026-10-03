//! Scalar completion keeps its actual exit and lease when destruction fails.

use async_trait::async_trait;
use run_domain::{RunOutcome, RunState};
use run_orchestrator::{RunOrchestrator, RunRepository};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::broadcast;
use vm_trait::{StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmSpec};

use super::super::support::{
    AutoExitProvider, CountingCompletion, MemoryRepository, MemoryVolumeStore,
    RecordingRuntimeManager, TestSpecFactory, lock, normal_stateless_command,
};

struct DestroyProvider {
    inner: AutoExitProvider,
    persistent: bool,
    attempts: Arc<AtomicUsize>,
    log: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl VmProvider for DestroyProvider {
    fn name(&self) -> &'static str {
        "completion-destroy-denial"
    }
    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(DestroyInstance {
            inner: self.inner.provision(spec).await?,
            persistent: self.persistent,
            attempts: Arc::clone(&self.attempts),
            log: Arc::clone(&self.log),
        }))
    }
    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Err(VmError::InvalidState(
            "no orphan observation in this fixture",
        ))
    }
}

struct DestroyInstance {
    inner: Arc<dyn VmInstance>,
    persistent: bool,
    attempts: Arc<AtomicUsize>,
    log: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl VmInstance for DestroyInstance {
    fn id(&self) -> &VmId {
        self.inner.id()
    }
    async fn start(&self) -> Result<(), VmError> {
        self.inner.start().await
    }
    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        self.inner.stop(mode).await
    }
    async fn wait(&self) -> Result<VmExit, VmError> {
        self.inner.wait().await
    }
    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.inner.subscribe_events()
    }
    async fn destroy(&self) -> Result<(), VmError> {
        lock(&self.log).push("destroy-attempt");
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        if self.persistent || attempt == 0 {
            Err(VmError::InvalidState("controlled destroy failure"))
        } else {
            self.inner.destroy().await
        }
    }
}

#[tokio::test]
async fn persistent_destroy_failure_preserves_known_exit_and_fenced_binding() {
    exercise(true).await;
}

#[tokio::test]
async fn successful_retry_releases_only_after_destroy_and_keeps_failed_outcome() {
    exercise(false).await;
}

async fn exercise(persistent: bool) {
    let mut command = normal_stateless_command();
    command.requires_state = true;
    let log = Arc::new(Mutex::new(Vec::new()));
    let repository = Arc::new(MemoryRepository::new(&command));
    let volumes = Arc::new(MemoryVolumeStore::new(
        command.instance_id,
        Arc::clone(&log),
    ));
    let completion = Arc::new(CountingCompletion::new(0));
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(DestroyProvider {
        inner: AutoExitProvider::new(Arc::clone(&log)),
        persistent,
        attempts: Arc::clone(&attempts),
        log: Arc::clone(&log),
    });
    let orchestrator = RunOrchestrator::new(
        repository.clone(),
        volumes.clone(),
        provider,
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_completion_observer(completion.clone())
    .with_runtime_manager(Arc::new(RecordingRuntimeManager {
        log: Arc::clone(&log),
    }));
    let result = orchestrator.start_run(&command).await;
    let run = repository.get(command.run_id).await.unwrap();
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert_eq!(
        run.exit,
        Some(VmExit {
            code: Some(0),
            signal: None
        })
    );
    assert_eq!(run.volume_id, Some(volumes.volume.id));
    assert_eq!(run.lease_id, Some(volumes.lease.id));
    assert_eq!(run.lease_fencing_token, Some(volumes.lease.fencing_token));
    assert!(
        run.failure
            .as_deref()
            .unwrap()
            .contains("controlled destroy failure")
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_completion_effects(persistent, &result, &run, &completion, &log);
}

fn assert_completion_effects(
    persistent: bool,
    result: &Result<run_domain::Run, run_orchestrator::OrchestratorError>,
    run: &run_domain::Run,
    completion: &CountingCompletion,
    log: &Mutex<Vec<&'static str>>,
) {
    let events = lock(log);
    if persistent {
        assert!(result.is_err());
        assert_eq!(run.state, RunState::CleaningUp);
        assert!(!events.contains(&"release"));
        assert!(!events.contains(&"runtime-destroy"));
        assert_eq!(completion.completions.load(Ordering::SeqCst), 0);
    } else {
        assert!(result.is_ok());
        assert_eq!(run.state, RunState::CleanedUp);
        let destroyed = events.iter().position(|e| *e == "destroy").unwrap();
        let released = events.iter().position(|e| *e == "release").unwrap();
        assert!(destroyed < released);
        assert_eq!(completion.completions.load(Ordering::SeqCst), 1);
    }
    drop(events);
}

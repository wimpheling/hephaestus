use async_trait::async_trait;
use run_domain::{Run, StartRun};
use run_orchestrator::{PreparedRunRuntime, RunOrchestrator, RunRuntimeError, RunRuntimeManager};
use runtime_types::{LeaseId, RunId, VolumeId};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Mutex as AsyncMutex;

use super::super::support::{
    MemoryRepository, MemoryVolumeStore, RecordingCompletion, RecordingRuntimeManager,
    TestSpecFactory, lock, normal_stateless_command,
};
use super::{
    provider::ScopedProvider,
    repository::{CleanupRepository, CleanupState},
    volumes::GlobalVolumes,
};

pub struct Fixture {
    pub command: StartRun,
    pub runs: Arc<MemoryRepository>,
    pub volumes: Arc<MemoryVolumeStore>,
    pub cleanup: Arc<CleanupRepository>,
    pub provider: Arc<ScopedProvider>,
    pub log: Arc<Mutex<Vec<&'static str>>>,
}

impl Fixture {
    pub fn new() -> Self {
        let command = normal_stateless_command();
        let log = Arc::new(Mutex::new(Vec::new()));
        let runs = Arc::new(MemoryRepository::new(&command));
        let volumes = Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        ));
        let cleanup = Arc::new(CleanupRepository {
            runs: Arc::clone(&runs),
            volumes: Arc::clone(&volumes),
            state: AsyncMutex::new(CleanupState::default()),
            log: Arc::clone(&log),
            planning_calls: AtomicUsize::new(0),
            pause_planning: Mutex::new(None),
        });
        Self {
            command,
            runs,
            volumes,
            cleanup,
            provider: Arc::new(ScopedProvider::new(Arc::clone(&log))),
            log,
        }
    }

    pub fn orchestrator(&self) -> RunOrchestrator {
        RunOrchestrator::new(
            self.runs.clone(),
            self.volumes.clone(),
            self.provider.clone(),
            Arc::new(TestSpecFactory),
            32 * 1024 * 1024,
        )
        .with_cleanup_repository(
            self.cleanup.clone(),
            Arc::new(GlobalVolumes(self.volumes.clone())),
        )
        .with_runtime_manager(Arc::new(RecordingRuntimeManager {
            log: Arc::clone(&self.log),
        }))
        .with_completion_observer(Arc::new(RecordingCompletion {
            log: Arc::clone(&self.log),
        }))
    }

    pub fn add_leases(&self, count: usize) {
        let mut leases = lock(&self.volumes.stale);
        for _ in 0..count {
            let mut lease = self.volumes.lease.clone();
            lease.id = LeaseId::new();
            lease.volume_id = VolumeId::new();
            lease.run_id = self.command.run_id;
            leases.push(lease);
        }
    }

    pub fn count(&self, name: &str) -> usize {
        lock(&self.log)
            .iter()
            .filter(|entry| **entry == name)
            .count()
    }
}

pub struct FailingTransient {
    pub fail: Arc<AtomicBool>,
    pub log: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunRuntimeManager for FailingTransient {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        Ok(PreparedRunRuntime::default())
    }
    async fn destroy(&self, _run: RunId) -> Result<(), RunRuntimeError> {
        lock(&self.log).push("transient-cleanup");
        if self.fail.load(Ordering::SeqCst) {
            return Err(RunRuntimeError::redacted("transient cleanup failed"));
        }
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        Ok(0)
    }
}

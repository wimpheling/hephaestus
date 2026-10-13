use super::super::{
    canonical::provider::ScopedProvider,
    support::{
        MemoryVolumeStore, RecordingAuthorityManager, RecordingCompletion,
        RecordingRuntimeGitWorkspaceManager, RecordingRuntimeManager, RecordingWorkspaceManager,
        TestSpecFactory, lock, normal_stateless_command,
    },
};
use super::{repository::Repository, volumes::Volumes};
use run_domain::{LegacyVmPlacementScope, StartRun};
use run_orchestrator::{RunOperationGuards, RunOrchestrator};
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use tokio::sync::Mutex as AsyncMutex;
use vm_trait::{VmProvider, VmProviderOwnerScope};
use volume_trait::ScalarLeaseHistory;

pub struct Fixture {
    pub command: StartRun,
    pub runs: Arc<Repository>,
    pub volumes: Arc<Volumes>,
    pub provider: Arc<ScopedProvider>,
    pub guards: Arc<RunOperationGuards>,
    pub log: Arc<Mutex<Vec<&'static str>>>,
}
impl Fixture {
    pub fn new(state: bool) -> Self {
        let mut command = normal_stateless_command();
        command.requires_state = state;
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(ScopedProvider::new(log.clone()));
        *lock(&provider.scope) =
            VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "test".into()).unwrap();
        let scope = LegacyVmPlacementScope::new(provider.owner_scope().unwrap()).unwrap();
        let runs = Arc::new(Repository::new(&command, scope));
        let volumes = Arc::new(Volumes {
            inner: MemoryVolumeStore::new(command.instance_id, log.clone()),
            runs: runs.clone(),
            history: AsyncMutex::new(ScalarLeaseHistory::NoHistory),
            fail_after_acquire: AtomicBool::new(false),
        });
        Self {
            command,
            runs,
            volumes,
            provider,
            guards: Arc::new(RunOperationGuards::default()),
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
        .with_legacy_vm_placement(self.runs.scope.clone())
        .with_operation_guards(self.guards.clone())
        .with_runtime_manager(Arc::new(RecordingRuntimeManager {
            log: self.log.clone(),
        }))
        .with_workspace_manager(Arc::new(RecordingWorkspaceManager {
            log: self.log.clone(),
        }))
        .with_authority_manager(Arc::new(RecordingAuthorityManager {
            log: self.log.clone(),
            reject_acknowledgement: false,
        }))
        .with_runtime_git_workspace_manager(Arc::new(RecordingRuntimeGitWorkspaceManager {
            log: self.log.clone(),
        }))
        .with_completion_observer(Arc::new(RecordingCompletion {
            log: self.log.clone(),
        }))
    }
    pub fn count(&self, event: &str) -> usize {
        lock(&self.log)
            .iter()
            .filter(|entry| **entry == event)
            .count()
    }
}

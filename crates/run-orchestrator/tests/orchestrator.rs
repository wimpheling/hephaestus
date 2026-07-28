//! Hardware-independent orchestration and cleanup-ordering coverage.

use async_trait::async_trait;
use run_domain::{CancelRun, Run, RunOutcome, RunState, StartRun};
use run_orchestrator::{
    CreateRunResult, OrchestratorError, OutboxRecord, RepositoryError, RunOrchestrator,
    RunRepository, StoredVmEvent, VmSpecFactory,
};
use runtime_types::{AgentId, CommandId, EventId, LeaseId, RunId, VolumeId};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, MutexGuard},
};
use time::OffsetDateTime;
use tokio::sync::{Mutex, broadcast, watch};
use uuid::Uuid;
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId,
    VmInstance, VmMount, VmProvider, VmResources, VmSpec,
};
use volume_trait::{
    AGENT_STATE_DISK_ID, Volume, VolumeAttachment, VolumeError, VolumeKind, VolumeLease,
    VolumeState, VolumeStore,
};
use workspace_domain::{
    PreparedWorkspace, PublishedResult, RunWorkspaceManager, WorkspaceError, WorkspaceId,
};

#[tokio::test]
async fn destroys_vm_before_releasing_lease_and_deduplicates_start() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        agent_id: AgentId::new(),
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let volume = Arc::new(MemoryVolumeStore::new(command.agent_id, Arc::clone(&log)));
    let provider = Arc::new(AutoExitProvider::new(Arc::clone(&log)));
    let orchestrator = RunOrchestrator::new(
        repository.clone(),
        volume,
        provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    );

    let run = orchestrator
        .start_run(&command)
        .await
        .expect("orchestrated run");
    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    assert!(
        repository
            .events
            .lock()
            .await
            .iter()
            .any(|event| event.event_type == "vm.exited"),
        "final VM event was not persisted"
    );
    let (destroyed, released) = {
        let entries = lock(&log);
        let destroyed = entries
            .iter()
            .position(|entry| *entry == "destroy")
            .expect("destroy event");
        let released = entries
            .iter()
            .position(|entry| *entry == "release")
            .expect("release event");
        drop(entries);
        (destroyed, released)
    };
    assert!(destroyed < released, "lease released before VM destruction");
    let spec = provider.spec().expect("provisioned spec");
    let disk = spec
        .disks
        .iter()
        .find(|disk| disk.id == AGENT_STATE_DISK_ID)
        .expect("agent-state disk");
    assert!(!disk.read_only);

    let before = lock(&log).len();
    let duplicate = orchestrator
        .start_run(&command)
        .await
        .expect("duplicate command");
    assert_eq!(duplicate.state, RunState::CleanedUp);
    assert_eq!(lock(&log).len(), before);
}

#[tokio::test]
async fn finalizes_workspace_only_after_vm_is_destroyed() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        agent_id: AgentId::new(),
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let workspaces = Arc::new(RecordingWorkspaceManager {
        log: Arc::clone(&log),
    });
    let orchestrator = RunOrchestrator::new(
        repository,
        Arc::new(MemoryVolumeStore::new(command.agent_id, Arc::clone(&log))),
        Arc::new(AutoExitProvider::new(Arc::clone(&log))),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_workspace_manager(workspaces);

    let run = orchestrator
        .start_run(&command)
        .await
        .expect("orchestrated workspace run");
    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    let entries = lock(&log);
    let destroy = entries
        .iter()
        .position(|entry| *entry == "destroy")
        .expect("destroy");
    let finalize = entries
        .iter()
        .position(|entry| *entry == "finalize")
        .expect("finalize");
    let release = entries
        .iter()
        .position(|entry| *entry == "release")
        .expect("release");
    drop(entries);
    assert!(destroy < finalize && finalize < release);
}

#[tokio::test]
async fn stale_recovery_fences_before_provider_cleanup_and_release() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        agent_id: AgentId::new(),
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    {
        let mut run = repository.run.lock().await;
        run.state = RunState::Running;
        run.vm_id = Some(run.id.to_string());
    }
    let volume = Arc::new(MemoryVolumeStore::new(command.agent_id, Arc::clone(&log)));
    let mut stale = volume.lease.clone();
    stale.run_id = command.run_id;
    stale.volume_id = volume.volume.id;
    lock(&volume.stale).push(stale);
    let provider = Arc::new(AutoExitProvider::new(Arc::clone(&log)));
    let orchestrator = RunOrchestrator::new(
        repository.clone(),
        volume,
        provider,
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    );

    assert_eq!(
        orchestrator
            .recover_stale_leases()
            .await
            .expect("recover stale lease"),
        1
    );
    assert_eq!(
        repository.get(command.run_id).await.unwrap().state,
        RunState::CleanedUp
    );
    let entries = lock(&log);
    let fenced = entries
        .iter()
        .position(|entry| *entry == "recover-begin")
        .expect("recovery fence");
    let cleaned = entries
        .iter()
        .position(|entry| *entry == "orphan-cleanup")
        .expect("provider cleanup");
    let released = entries
        .iter()
        .position(|entry| *entry == "recover-finish")
        .expect("recovery release");
    drop(entries);
    assert!(fenced < cleaned && cleaned < released);
}

#[tokio::test]
async fn duplicate_in_flight_start_waits_for_reconciliation() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        agent_id: AgentId::new(),
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    *repository.created.lock().await = true;
    repository.run.lock().await.state = RunState::Provisioning;
    let orchestrator = RunOrchestrator::new(
        repository,
        Arc::new(MemoryVolumeStore::new(command.agent_id, Arc::clone(&log))),
        Arc::new(AutoExitProvider::new(Arc::clone(&log))),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    );

    assert!(matches!(
        orchestrator.start_run(&command).await,
        Err(OrchestratorError::RunInProgress(id)) if id == command.run_id
    ));
    assert!(lock(&log).is_empty(), "duplicate provisioned another VM");
}

#[tokio::test]
async fn restart_finishes_cleanup_after_lease_was_already_released() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        agent_id: AgentId::new(),
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    {
        let mut run = repository.run.lock().await;
        run.state = RunState::CleaningUp;
        run.outcome = Some(RunOutcome::Succeeded);
        run.vm_id = Some(run.id.to_string());
    }
    let orchestrator = RunOrchestrator::new(
        repository.clone(),
        Arc::new(MemoryVolumeStore::new(command.agent_id, Arc::clone(&log))),
        Arc::new(AutoExitProvider::new(Arc::clone(&log))),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    );

    assert_eq!(
        orchestrator
            .recover_after_restart()
            .await
            .expect("restart recovery"),
        1
    );
    let recovered = repository.get(command.run_id).await.unwrap();
    assert_eq!(recovered.state, RunState::CleanedUp);
    assert_eq!(recovered.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(lock(&log).as_slice(), ["orphan-cleanup"]);
}

struct TestSpecFactory;

#[async_trait]
impl VmSpecFactory for TestSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        Ok(VmSpec {
            id: VmId(run.id.to_string()),
            root: RootFilesystem::Directory {
                host_path: PathBuf::from("/fake/root"),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 128,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/bin/true"),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: None,
            },
            labels: BTreeMap::new(),
        })
    }
}

struct RecordingWorkspaceManager {
    log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunWorkspaceManager for RecordingWorkspaceManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedWorkspace, WorkspaceError> {
        lock(&self.log).push("prepare");
        Ok(PreparedWorkspace {
            id: Some(WorkspaceId::new()),
            mounts: vec![VmMount {
                tag: String::from("repository-work"),
                host_path: PathBuf::from("/fake/work"),
                guest_path: PathBuf::from("/workspace/work"),
                read_only: false,
            }],
        })
    }

    async fn finalize(
        &self,
        _run: &Run,
        _message: &str,
    ) -> Result<Option<PublishedResult>, WorkspaceError> {
        lock(&self.log).push("finalize");
        Ok(None)
    }

    async fn abandon(&self, _run_id: RunId) -> Result<(), WorkspaceError> {
        lock(&self.log).push("abandon");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, WorkspaceError> {
        Ok(0)
    }
}

struct MemoryRepository {
    run: Mutex<Run>,
    created: Mutex<bool>,
    events: Mutex<Vec<StoredVmEvent>>,
}

impl MemoryRepository {
    fn new(command: &StartRun) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            run: Mutex::new(Run {
                id: command.run_id,
                agent_id: command.agent_id,
                command_id: command.command_id,
                volume_id: None,
                lease_id: None,
                vm_id: None,
                state: RunState::Queued,
                outcome: None,
                exit: None,
                failure: None,
                cancel_requested_at: None,
                created_at: now,
                updated_at: now,
                state_version: 0,
            }),
            created: Mutex::new(false),
            events: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl RunRepository for MemoryRepository {
    async fn initialize(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn create_run(&self, _command: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        let mut created = self.created.lock().await;
        let was_created = !*created;
        *created = true;
        drop(created);
        Ok(CreateRunResult {
            run: self.run.lock().await.clone(),
            created: was_created,
        })
    }

    async fn get(&self, _run_id: RunId) -> Result<Run, RepositoryError> {
        Ok(self.run.lock().await.clone())
    }

    async fn bind_resources(
        &self,
        _run_id: RunId,
        volume_id: VolumeId,
        lease_id: LeaseId,
        vm_id: &str,
    ) -> Result<Run, RepositoryError> {
        let mut run = self.run.lock().await;
        run.volume_id = Some(volume_id);
        run.lease_id = Some(lease_id);
        run.vm_id = Some(vm_id.to_owned());
        Ok(run.clone())
    }

    async fn transition(
        &self,
        _run_id: RunId,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        let mut run = self.run.lock().await;
        if !run.state.can_transition_to(next) {
            return Err(RepositoryError::InvalidTransition(
                run_domain::InvalidTransition {
                    current: run.state,
                    requested: next,
                },
            ));
        }
        run.state = next;
        run.outcome = next.outcome().or(run.outcome);
        run.exit = exit.cloned().or_else(|| run.exit.clone());
        run.failure = failure.map(str::to_owned).or_else(|| run.failure.clone());
        run.state_version += 1;
        run.updated_at = OffsetDateTime::now_utc();
        Ok(run.clone())
    }

    async fn append_vm_event(
        &self,
        _run_id: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError> {
        self.events.lock().await.push(event);
        Ok(())
    }

    async fn request_cancel(&self, _command: &CancelRun) -> Result<bool, RepositoryError> {
        self.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
        Ok(true)
    }

    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        Ok(vec![self.run.lock().await.clone()])
    }

    async fn unpublished_outbox(&self, _limit: i64) -> Result<Vec<OutboxRecord>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn mark_outbox_published(&self, _event_id: EventId) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn mark_outbox_failed(
        &self,
        _event_id: EventId,
        _error: &str,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }
}

struct MemoryVolumeStore {
    volume: Volume,
    lease: VolumeLease,
    log: Arc<StdMutex<Vec<&'static str>>>,
    stale: StdMutex<Vec<VolumeLease>>,
}

impl MemoryVolumeStore {
    fn new(agent_id: AgentId, log: Arc<StdMutex<Vec<&'static str>>>) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            volume: Volume {
                id: VolumeId::new(),
                agent_id,
                kind: VolumeKind::AgentState,
                host_id: String::from("test"),
                host_path: PathBuf::from("/fake/agent-state.raw"),
                capacity_bytes: 32 * 1024 * 1024,
                filesystem_uuid: Uuid::new_v4(),
                state: VolumeState::Ready,
                key_reference: None,
                encryption_version: None,
                backup_revision: None,
                checksum: None,
                last_successful_backup_at: None,
            },
            lease: VolumeLease {
                id: LeaseId::new(),
                volume_id: VolumeId::new(),
                run_id: RunId::new(),
                host_id: String::from("test"),
                fencing_token: 1,
                acquired_at: now,
                heartbeat_at: now,
                expires_at: now + time::Duration::minutes(1),
                attached_at: None,
            },
            log,
            stale: StdMutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl VolumeStore for MemoryVolumeStore {
    async fn resolve_agent_state(
        &self,
        _agent_id: AgentId,
        _capacity_bytes: u64,
    ) -> Result<Volume, VolumeError> {
        Ok(self.volume.clone())
    }

    async fn acquire(
        &self,
        volume_id: VolumeId,
        run_id: RunId,
    ) -> Result<VolumeAttachment, VolumeError> {
        let mut lease = self.lease.clone();
        lease.volume_id = volume_id;
        lease.run_id = run_id;
        Ok(VolumeAttachment {
            volume: self.volume.clone(),
            lease,
            disk_id: AGENT_STATE_DISK_ID,
        })
    }

    async fn mark_attached(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        lock(&self.log).push("attached");
        let mut attached = lease.clone();
        attached.attached_at = Some(OffsetDateTime::now_utc());
        Ok(attached)
    }

    async fn heartbeat(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        Ok(lease.clone())
    }

    async fn active_lease_for_run(
        &self,
        run_id: RunId,
    ) -> Result<Option<VolumeLease>, VolumeError> {
        Ok(lock(&self.stale)
            .iter()
            .find(|lease| lease.run_id == run_id)
            .cloned())
    }

    async fn release_after_detach(&self, _lease: &VolumeLease) -> Result<(), VolumeError> {
        lock(&self.log).push("release");
        Ok(())
    }

    async fn stale_leases(&self, _now: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        Ok(lock(&self.stale).clone())
    }

    async fn begin_recovery(&self, _lease: &VolumeLease) -> Result<(), VolumeError> {
        lock(&self.log).push("recover-begin");
        Ok(())
    }

    async fn finish_recovery(&self, _lease: &VolumeLease) -> Result<(), VolumeError> {
        lock(&self.log).push("recover-finish");
        Ok(())
    }
}

struct AutoExitProvider {
    log: Arc<StdMutex<Vec<&'static str>>>,
    spec: StdMutex<Option<VmSpec>>,
}

impl AutoExitProvider {
    const fn new(log: Arc<StdMutex<Vec<&'static str>>>) -> Self {
        Self {
            log,
            spec: StdMutex::new(None),
        }
    }

    fn spec(&self) -> Option<VmSpec> {
        lock(&self.spec).clone()
    }
}

#[async_trait]
impl VmProvider for AutoExitProvider {
    fn name(&self) -> &'static str {
        "auto-exit"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        lock(&self.log).push("provision");
        *lock(&self.spec) = Some(spec.clone());
        Ok(Arc::new(AutoExitInstance::new(
            spec.id,
            Arc::clone(&self.log),
        )))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        lock(&self.log).push("orphan-cleanup");
        Ok(())
    }
}

struct AutoExitInstance {
    id: VmId,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
    log: Arc<StdMutex<Vec<&'static str>>>,
}

impl AutoExitInstance {
    fn new(id: VmId, log: Arc<StdMutex<Vec<&'static str>>>) -> Self {
        let (events, _) = broadcast::channel(8);
        let (exit, _) = watch::channel(None);
        Self {
            id,
            events,
            exit,
            log,
        }
    }
}

#[async_trait]
impl VmInstance for AutoExitInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        lock(&self.log).push("start");
        let _started = self.events.send(VmEvent::Started {
            ingress: Vec::new(),
        });
        let _ready = self.events.send(VmEvent::Ready);
        let _finalize = self.events.send(VmEvent::FinalizeResult {
            message: String::from("test result"),
        });
        let exit = VmExit {
            code: Some(0),
            signal: None,
        };
        self.exit.send_replace(Some(exit.clone()));
        let _exited = self.events.send(VmEvent::Exited(exit));
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let current = receiver.borrow_and_update().clone();
            if let Some(exit) = current {
                return Ok(exit);
            }
            receiver
                .changed()
                .await
                .map_err(|_| VmError::InvalidState("test exit channel closed"))?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        lock(&self.log).push("destroy");
        Ok(())
    }
}

fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

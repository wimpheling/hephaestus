//! Hardware-independent orchestration and cleanup-ordering coverage.

use async_trait::async_trait;
use run_domain::{CancelRun, Run, RunKind, RunOutcome, RunState, StartRun};
use run_orchestrator::{
    CreateRunResult, OrchestratorError, PreparedRunAuthority, PreparedRunRuntime,
    PreparedRunSecrets, RepositoryError, RunAuthorityError, RunAuthorityManager,
    RunAuthorizationError, RunCompletionError, RunCompletionObserver, RunLaunchAuthorizer,
    RunOrchestrator, RunRepository, RunRuntimeError, RunRuntimeManager, RunSecretError,
    RunSecretManager, StoredVmEvent, VmSpecFactory,
};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, LeaseId,
    ReleaseAgentId, ReleaseId, RunId, VolumeId,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};
use time::OffsetDateTime;
use tokio::sync::{Mutex, broadcast, watch};
use uuid::Uuid;
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, RuntimeAuthorityBootstrap, StopMode, VmError,
    VmEvent, VmExit, VmId, VmInstance, VmMount, VmProvider, VmResources, VmSpec,
};
use volume_trait::{
    INSTANCE_STATE_DISK_ID, Volume, VolumeAttachment, VolumeError, VolumeKind, VolumeLease,
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
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let volume = Arc::new(MemoryVolumeStore::new(
        command.instance_id,
        Arc::clone(&log),
    ));
    let provider = Arc::new(AutoExitProvider::new(Arc::clone(&log)));
    let orchestrator = RunOrchestrator::new(
        repository.clone(),
        volume,
        provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_launch_authorizer(Arc::new(RecordingLaunchAuthorizer {
        log: Arc::clone(&log),
    }))
    .with_runtime_manager(Arc::new(RecordingRuntimeManager {
        log: Arc::clone(&log),
    }))
    .with_authority_manager(Arc::new(RecordingAuthorityManager {
        log: Arc::clone(&log),
        reject_acknowledgement: false,
    }));

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
    let (destroyed, runtime_destroyed, released) = {
        let entries = lock(&log);
        let destroyed = entries
            .iter()
            .position(|entry| *entry == "destroy")
            .expect("destroy event");
        let runtime_destroyed = entries
            .iter()
            .position(|entry| *entry == "runtime-destroy")
            .expect("runtime destroy event");
        let released = entries
            .iter()
            .position(|entry| *entry == "release")
            .expect("release event");
        drop(entries);
        (destroyed, runtime_destroyed, released)
    };
    assert!(
        destroyed < runtime_destroyed && runtime_destroyed < released,
        "runtime or lease cleanup happened before VM destruction"
    );
    assert_launch_order(&log);
    let spec = provider.spec().expect("provisioned spec");
    let disk = spec
        .disks
        .iter()
        .find(|disk| disk.id == INSTANCE_STATE_DISK_ID)
        .expect("instance-state disk");
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
async fn missing_runtime_authority_acknowledgement_destroys_guest_before_revocation() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = normal_stateless_command();
    let provider = Arc::new(AutoExitProvider::new(Arc::clone(&log)));
    let orchestrator = RunOrchestrator::new(
        Arc::new(MemoryRepository::new(&command)),
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
        provider,
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_authority_manager(Arc::new(RecordingAuthorityManager {
        log: Arc::clone(&log),
        reject_acknowledgement: true,
    }));

    let run = orchestrator
        .start_run(&command)
        .await
        .expect("acknowledgement failure is durably cleaned");

    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    let entries = lock(&log);
    let started = entries
        .iter()
        .position(|entry| *entry == "start")
        .expect("guest start");
    let acknowledgement = entries
        .iter()
        .position(|entry| *entry == "authority-ack")
        .expect("authority acknowledgement");
    let destroyed = entries
        .iter()
        .position(|entry| *entry == "destroy")
        .expect("guest destruction");
    let revoked = entries
        .iter()
        .position(|entry| *entry == "authority-revoke")
        .expect("authority revocation");
    drop(entries);
    assert!(started < acknowledgement);
    assert!(acknowledgement < destroyed && destroyed < revoked);
}

#[tokio::test]
async fn denied_live_authority_prevents_artifact_preparation_and_vm_provision() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let provider = Arc::new(AutoExitProvider::new(Arc::clone(&log)));
    let orchestrator = RunOrchestrator::new(
        repository,
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
        provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_launch_authorizer(Arc::new(DenyLaunchAuthorizer))
    .with_runtime_manager(Arc::new(RecordingRuntimeManager {
        log: Arc::clone(&log),
    }));

    let run = orchestrator
        .start_run(&command)
        .await
        .expect("denial is a durable failed run");

    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert!(provider.spec().is_none());
    assert!(!lock(&log).contains(&"runtime-prepare"));
}

#[tokio::test]
async fn revocation_after_start_allows_current_guest_but_blocks_warm_cache_replay() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let revoked = Arc::new(AtomicBool::new(false));
    let authorizer = Arc::new(RevocableLaunchAuthorizer {
        revoked: Arc::clone(&revoked),
        log: Arc::clone(&log),
    });
    let provider = Arc::new(RevokeOnProvisionProvider {
        inner: AutoExitProvider::new(Arc::clone(&log)),
        revoked: Arc::clone(&revoked),
    });
    let first = normal_stateless_command();
    let first_orchestrator = RunOrchestrator::new(
        Arc::new(MemoryRepository::new(&first)),
        Arc::new(MemoryVolumeStore::new(first.instance_id, Arc::clone(&log))),
        provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_launch_authorizer(authorizer.clone())
    .with_runtime_manager(Arc::new(RecordingRuntimeManager {
        log: Arc::clone(&log),
    }));
    let completed = first_orchestrator
        .start_run(&first)
        .await
        .expect("already provisioned guest may finish");
    assert_eq!(completed.outcome, Some(RunOutcome::Succeeded));
    assert!(revoked.load(Ordering::SeqCst));

    let first_prepares = lock(&log)
        .iter()
        .filter(|event| **event == "runtime-prepare")
        .count();
    let first_provisions = lock(&log)
        .iter()
        .filter(|event| **event == "provision")
        .count();
    assert_eq!((first_prepares, first_provisions), (1, 1));

    let second = normal_stateless_command();
    let second_orchestrator = RunOrchestrator::new(
        Arc::new(MemoryRepository::new(&second)),
        Arc::new(MemoryVolumeStore::new(second.instance_id, Arc::clone(&log))),
        provider,
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_launch_authorizer(authorizer)
    .with_runtime_manager(Arc::new(RecordingRuntimeManager {
        log: Arc::clone(&log),
    }));
    let denied = second_orchestrator
        .start_run(&second)
        .await
        .expect("revoked replay is durably denied");
    assert_eq!(denied.outcome, Some(RunOutcome::Failed));
    assert_eq!(
        lock(&log)
            .iter()
            .filter(|event| **event == "runtime-prepare")
            .count(),
        first_prepares,
        "warm artifact state must not bypass live authorization"
    );
    assert_eq!(
        lock(&log)
            .iter()
            .filter(|event| **event == "provision")
            .count(),
        first_provisions
    );
}

#[tokio::test]
async fn revoked_secret_after_materialization_prevents_vm_provision_and_cleans() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let provider = Arc::new(AutoExitProvider::new(Arc::clone(&log)));
    let orchestrator = RunOrchestrator::new(
        repository,
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
        provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_secret_manager(Arc::new(RevokeBeforeProvisionSecrets {
        log: Arc::clone(&log),
    }))
    .with_runtime_manager(Arc::new(RecordingRuntimeManager {
        log: Arc::clone(&log),
    }));

    let run = orchestrator
        .start_run(&command)
        .await
        .expect("secret revocation is a durable failed run");

    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert!(provider.spec().is_none());
    let entries = lock(&log);
    let prepared = entries
        .iter()
        .position(|entry| *entry == "secret-prepare")
        .expect("secret materialized");
    let denied = entries
        .iter()
        .position(|entry| *entry == "secret-reauthorize")
        .expect("secret reauthorization");
    let destroyed = entries
        .iter()
        .position(|entry| *entry == "secret-destroy")
        .expect("secret cleanup");
    drop(entries);
    assert!(prepared < denied && denied < destroyed);
}

#[tokio::test]
async fn stateless_run_never_acquires_or_mounts_instance_state() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let provider = Arc::new(AutoExitProvider::new(Arc::clone(&log)));
    let orchestrator = RunOrchestrator::new(
        repository.clone(),
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
        provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    );

    let run = orchestrator
        .start_run(&command)
        .await
        .expect("orchestrated stateless run");

    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    assert!(run.volume_id.is_none());
    assert!(run.lease_id.is_none());
    assert!(
        provider
            .spec()
            .expect("provisioned spec")
            .disks
            .iter()
            .all(|disk| disk.id != INSTANCE_STATE_DISK_ID)
    );
    assert!(!lock(&log).contains(&"attached"));
    assert!(!lock(&log).contains(&"release"));
}

#[tokio::test]
async fn observes_update_result_only_after_state_lease_release() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: None,
        kind: RunKind::Update,
        requires_state: true,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let orchestrator = RunOrchestrator::new(
        repository,
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
        Arc::new(AutoExitProvider::new(Arc::clone(&log))),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_completion_observer(Arc::new(RecordingCompletion {
        log: Arc::clone(&log),
    }));

    let run = orchestrator
        .start_run(&command)
        .await
        .expect("orchestrated update run");

    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    let entries = lock(&log);
    let release = entries
        .iter()
        .position(|entry| *entry == "release")
        .expect("lease release");
    let completion = entries
        .iter()
        .position(|entry| *entry == "completion")
        .expect("completion observation");
    drop(entries);
    assert!(release < completion);
}

#[tokio::test(start_paused = true)]
async fn update_wall_clock_timeout_fails_and_cleans_before_observation() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: None,
        kind: RunKind::Update,
        requires_state: true,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let orchestrator = RunOrchestrator::new(
        repository,
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
        Arc::new(HangingProvider {
            log: Arc::clone(&log),
        }),
        Arc::new(TimeoutSpecFactory),
        32 * 1024 * 1024,
    )
    .with_completion_observer(Arc::new(RecordingCompletion {
        log: Arc::clone(&log),
    }));

    let run = orchestrator
        .start_run(&command)
        .await
        .expect("timed-out update cleanup");

    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert_eq!(
        run.failure.as_deref(),
        Some("guest wall-clock timeout elapsed")
    );
    let entries = lock(&log);
    for expected in ["stop", "destroy", "release", "completion"] {
        assert!(
            entries.contains(&expected),
            "missing {expected}: {entries:?}"
        );
    }
    let release = entries
        .iter()
        .position(|entry| *entry == "release")
        .expect("lease release");
    let completion = entries
        .iter()
        .position(|entry| *entry == "completion")
        .expect("completion observation");
    drop(entries);
    assert!(release < completion);
}

#[tokio::test]
async fn finalizes_workspace_only_after_vm_is_destroyed() {
    let log = Arc::new(StdMutex::new(Vec::new()));
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    let workspaces = Arc::new(RecordingWorkspaceManager {
        log: Arc::clone(&log),
    });
    let orchestrator = RunOrchestrator::new(
        repository,
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
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
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    {
        let mut run = repository.run.lock().await;
        run.state = RunState::Running;
        run.vm_id = Some(run.id.to_string());
    }
    let volume = Arc::new(MemoryVolumeStore::new(
        command.instance_id,
        Arc::clone(&log),
    ));
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
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    };
    let repository = Arc::new(MemoryRepository::new(&command));
    *repository.created.lock().await = true;
    repository.run.lock().await.state = RunState::Provisioning;
    let orchestrator = RunOrchestrator::new(
        repository,
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
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
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
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
        Arc::new(MemoryVolumeStore::new(
            command.instance_id,
            Arc::clone(&log),
        )),
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
            runtime_authority: None,
            labels: BTreeMap::new(),
        })
    }
}

struct TimeoutSpecFactory;

#[async_trait]
impl VmSpecFactory for TimeoutSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let mut spec = TestSpecFactory.build(run).await?;
        spec.labels.insert(
            String::from("hephaestus.wall-clock-timeout-seconds"),
            String::from("1"),
        );
        Ok(spec)
    }
}

struct RecordingWorkspaceManager {
    log: Arc<StdMutex<Vec<&'static str>>>,
}

struct RecordingRuntimeManager {
    log: Arc<StdMutex<Vec<&'static str>>>,
}

struct RecordingAuthorityManager {
    log: Arc<StdMutex<Vec<&'static str>>>,
    reject_acknowledgement: bool,
}

#[async_trait]
impl RunAuthorityManager for RecordingAuthorityManager {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunAuthority, RunAuthorityError> {
        lock(&self.log).push("authority-prepare");
        Ok(PreparedRunAuthority {
            bootstrap: Some(RuntimeAuthorityBootstrap::new(
                run.id.as_uuid(),
                1,
                [0x5A; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
            )),
        })
    }

    async fn reauthorize(&self, _run: &Run) -> Result<(), RunAuthorityError> {
        lock(&self.log).push("authority-reauthorize");
        Ok(())
    }

    async fn acknowledge(
        &self,
        _run: &Run,
        _session_id: Uuid,
        _generation: u64,
    ) -> Result<(), RunAuthorityError> {
        lock(&self.log).push("authority-ack");
        if self.reject_acknowledgement {
            Err(RunAuthorityError::redacted(
                "exact issuance generation was not acknowledged",
            ))
        } else {
            Ok(())
        }
    }

    async fn revoke_after_guest(&self, _run_id: RunId) -> Result<(), RunAuthorityError> {
        lock(&self.log).push("authority-revoke");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunAuthorityError> {
        lock(&self.log).push("authority-recover");
        Ok(0)
    }
}

struct RecordingLaunchAuthorizer {
    log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunLaunchAuthorizer for RecordingLaunchAuthorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        lock(&self.log).push("authorize");
        Ok(())
    }
}

struct DenyLaunchAuthorizer;

#[async_trait]
impl RunLaunchAuthorizer for DenyLaunchAuthorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        Err(RunAuthorizationError::redacted("denied by test policy"))
    }
}

struct RevocableLaunchAuthorizer {
    revoked: Arc<AtomicBool>,
    log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunLaunchAuthorizer for RevocableLaunchAuthorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        lock(&self.log).push("authorize");
        if self.revoked.load(Ordering::SeqCst) {
            Err(RunAuthorizationError::redacted("revoked by test policy"))
        } else {
            Ok(())
        }
    }
}

struct RevokeBeforeProvisionSecrets {
    log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunSecretManager for RevokeBeforeProvisionSecrets {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        lock(&self.log).push("secret-prepare");
        Ok(PreparedRunSecrets::default())
    }

    async fn reauthorize(&self, _run: &Run) -> Result<(), RunSecretError> {
        lock(&self.log).push("secret-reauthorize");
        Err(RunSecretError::redacted("revoked by test"))
    }

    async fn destroy_after_guest(&self, _run_id: RunId) -> Result<(), RunSecretError> {
        lock(&self.log).push("secret-destroy");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunSecretError> {
        Ok(0)
    }
}

struct RecordingCompletion {
    log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunCompletionObserver for RecordingCompletion {
    async fn after_cleanup(&self, _run: &Run) -> Result<(), RunCompletionError> {
        lock(&self.log).push("completion");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        Ok(0)
    }
}

#[async_trait]
impl RunRuntimeManager for RecordingRuntimeManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        lock(&self.log).push("runtime-prepare");
        Ok(PreparedRunRuntime::default())
    }

    async fn destroy(&self, _run_id: RunId) -> Result<(), RunRuntimeError> {
        lock(&self.log).push("runtime-destroy");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        Ok(0)
    }
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
                instance_id: command.instance_id,
                instance_revision_id: command.instance_revision_id,
                release_id: command.release_id,
                release_agent_id: command.release_agent_id,
                attachment_id: command.attachment_id,
                kind: command.kind,
                command_id: command.command_id,
                requires_state: command.requires_state,
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
        volume_id: Option<VolumeId>,
        lease_id: Option<LeaseId>,
        vm_id: &str,
    ) -> Result<Run, RepositoryError> {
        let mut run = self.run.lock().await;
        run.volume_id = volume_id;
        run.lease_id = lease_id;
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
}

struct MemoryVolumeStore {
    volume: Volume,
    lease: VolumeLease,
    log: Arc<StdMutex<Vec<&'static str>>>,
    stale: StdMutex<Vec<VolumeLease>>,
}

impl MemoryVolumeStore {
    fn new(instance_id: AgentInstanceId, log: Arc<StdMutex<Vec<&'static str>>>) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            volume: Volume {
                id: VolumeId::new(),
                instance_id,
                kind: VolumeKind::InstanceState,
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
    async fn resolve_instance_state(
        &self,
        _agent_id: AgentInstanceId,
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
            disk_id: INSTANCE_STATE_DISK_ID,
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

struct RevokeOnProvisionProvider {
    inner: AutoExitProvider,
    revoked: Arc<AtomicBool>,
}

#[async_trait]
impl VmProvider for RevokeOnProvisionProvider {
    fn name(&self) -> &'static str {
        "revoke-on-provision"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        let instance = self.inner.provision(spec).await?;
        self.revoked.store(true, Ordering::SeqCst);
        Ok(instance)
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        self.inner.cleanup_orphan(id).await
    }
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
        let runtime_authority = spec
            .runtime_authority
            .as_ref()
            .map(|authority| (authority.session_id(), authority.generation()));
        *lock(&self.spec) = Some(spec.clone());
        Ok(Arc::new(AutoExitInstance::new(
            spec.id,
            Arc::clone(&self.log),
            runtime_authority,
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
    runtime_authority: Option<(Uuid, u64)>,
}

struct HangingProvider {
    log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl VmProvider for HangingProvider {
    fn name(&self) -> &'static str {
        "hanging"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(HangingInstance {
            id: spec.id,
            log: Arc::clone(&self.log),
            events: broadcast::channel(8).0,
        }))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct HangingInstance {
    id: VmId,
    log: Arc<StdMutex<Vec<&'static str>>>,
    events: broadcast::Sender<VmEvent>,
}

#[async_trait]
impl VmInstance for HangingInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        lock(&self.log).push("stop");
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        std::future::pending().await
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        lock(&self.log).push("destroy");
        Ok(())
    }
}

impl AutoExitInstance {
    fn new(
        id: VmId,
        log: Arc<StdMutex<Vec<&'static str>>>,
        runtime_authority: Option<(Uuid, u64)>,
    ) -> Self {
        let (events, _) = broadcast::channel(8);
        let (exit, _) = watch::channel(None);
        Self {
            id,
            events,
            exit,
            log,
            runtime_authority,
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
        if let Some((session_id, generation)) = self.runtime_authority {
            let _acknowledgement = self.events.send(VmEvent::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            });
        }
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

fn normal_stateless_command() -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    }
}

fn assert_launch_order(log: &StdMutex<Vec<&'static str>>) {
    let entries = lock(log);
    let authorization = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (*entry == "authorize").then_some(index))
        .collect::<Vec<_>>();
    let runtime_prepare = entries
        .iter()
        .position(|entry| *entry == "runtime-prepare")
        .expect("runtime preparation");
    let authority_prepare = entries
        .iter()
        .position(|entry| *entry == "authority-prepare")
        .expect("authority preparation");
    let authority_reauthorize = entries
        .iter()
        .position(|entry| *entry == "authority-reauthorize")
        .expect("authority reauthorization");
    let provision = entries
        .iter()
        .position(|entry| *entry == "provision")
        .expect("VM provision");
    drop(entries);
    assert_eq!(authorization.len(), 2);
    assert!(authorization[0] < runtime_prepare);
    assert!(runtime_prepare < authority_prepare);
    assert!(authority_prepare < authorization[1]);
    assert!(authorization[1] < authority_reauthorize && authority_reauthorize < provision);
}

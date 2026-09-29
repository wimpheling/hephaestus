use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};

use run_domain::{RunKind, RunOutcome, RunState, StartRun};
use run_orchestrator::RunOrchestrator;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use volume_trait::INSTANCE_STATE_DISK_ID;

use super::support::{
    AutoExitProvider, DenyLaunchAuthorizer, MemoryRepository, MemoryVolumeStore,
    RecordingAuthorityManager, RecordingLaunchAuthorizer, RecordingResourceObserver,
    RecordingRuntimeGitWorkspaceManager, RecordingRuntimeManager, TestSpecFactory,
    assert_launch_order, lock, normal_stateless_command,
};

#[tokio::test]
// This integration fixture keeps launch, guest cleanup, and duplicate-start
// assertions together so the ordering is checked against one run trace.
#[allow(clippy::too_many_lines)]
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
    .with_resource_observer(Arc::new(RecordingResourceObserver {
        log: Arc::clone(&log),
    }))
    .with_runtime_manager(Arc::new(RecordingRuntimeManager {
        log: Arc::clone(&log),
    }))
    .with_runtime_git_workspace_manager(Arc::new(RecordingRuntimeGitWorkspaceManager {
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
    let (destroyed, runtime_git_abandoned, runtime_destroyed, released) = {
        let entries = lock(&log);
        let destroyed = entries
            .iter()
            .position(|entry| *entry == "destroy")
            .expect("destroy event");
        let runtime_destroyed = entries
            .iter()
            .position(|entry| *entry == "runtime-destroy")
            .expect("runtime destroy event");
        let runtime_git_abandoned = entries
            .iter()
            .position(|entry| *entry == "runtime-git-abandon")
            .expect("runtime Git abandon event");
        let released = entries
            .iter()
            .position(|entry| *entry == "release")
            .expect("release event");
        drop(entries);
        (
            destroyed,
            runtime_git_abandoned,
            runtime_destroyed,
            released,
        )
    };
    assert!(
        destroyed < runtime_git_abandoned
            && runtime_git_abandoned < runtime_destroyed
            && runtime_destroyed < released,
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
    assert_eq!(
        spec.command.working_dir,
        Some(PathBuf::from("/workspace/git"))
    );
    assert_eq!(
        spec.runtime_git_bridge
            .expect("runtime Git bridge")
            .remote_url(),
        "http://127.0.0.1:19100/00000000-0000-0000-0000-000000000001"
    );
    assert!(
        spec.mounts
            .iter()
            .any(|mount| mount.tag == "runtime-git-worktree")
    );

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

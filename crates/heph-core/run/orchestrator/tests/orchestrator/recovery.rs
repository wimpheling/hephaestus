use std::sync::{Arc, Mutex as StdMutex};

use run_domain::{RunKind, RunOutcome, RunState, StartRun};
use run_orchestrator::{OrchestratorError, RunOrchestrator, RunRepository};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};

use super::support::{
    AutoExitProvider, HangingProvider, MemoryRepository, MemoryVolumeStore, RecordingCompletion,
    RecordingWorkspaceManager, TestSpecFactory, TimeoutSpecFactory, lock,
};

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

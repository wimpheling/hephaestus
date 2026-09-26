use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};
use volume_trait::INSTANCE_STATE_DISK_ID;

use run_domain::{RunKind, RunOutcome, RunState, StartRun};
use run_orchestrator::RunOrchestrator;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};

use super::support::{
    AutoExitProvider, MemoryRepository, MemoryVolumeStore, RecordingRuntimeManager,
    RevocableLaunchAuthorizer, RevokeBeforeProvisionSecrets, RevokeOnProvisionProvider,
    TestSpecFactory, lock, normal_stateless_command,
};

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

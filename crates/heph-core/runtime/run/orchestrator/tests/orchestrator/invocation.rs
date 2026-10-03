use std::sync::{Arc, Mutex};

use run_domain::{RunKind, StartRun};
use run_orchestrator::{OrchestratorError, RunOrchestrator};
use runtime_types::AgentAttachmentId;

use super::support::{
    AutoExitProvider, MemoryVolumeStore, RecordingAuthorityManager, RecordingLaunchAuthorizer,
    RecordingResourceObserver, RecordingRuntimeGitWorkspaceManager, RecordingRuntimeManager,
    RecordingWorkspaceManager, lock, normal_stateless_command,
};

#[path = "invocation_repository.rs"]
mod repository;

#[test]
fn run_kind_wire_values_remain_distinct_and_compatible() {
    for (kind, wire) in [
        (RunKind::Normal, "normal"),
        (RunKind::Update, "update"),
        (RunKind::Invocation, "invocation"),
    ] {
        let encoded = serde_json::to_value(kind).expect("serialize kind");
        assert_eq!(encoded, wire);
        assert_eq!(serde_json::from_value::<RunKind>(encoded).unwrap(), kind);
    }
    assert!(serde_json::from_str::<RunKind>("\"invoke\"").is_err());
}

#[test]
fn invocation_command_roundtrips_without_attachment_or_git_target() {
    let mut command = normal_stateless_command();
    command.kind = RunKind::Invocation;
    command.attachment_id = None;
    let encoded = serde_json::to_value(&command).expect("serialize command");
    assert_eq!(encoded["kind"], "invocation");
    assert!(encoded["attachment_id"].is_null());
    for field in ["git_ref", "commit_sha", "parameters", "runtime_policy"] {
        assert!(encoded.get(field).is_none());
    }
    assert_eq!(
        serde_json::from_value::<StartRun>(encoded).unwrap(),
        command
    );
}

#[tokio::test]
async fn invocation_is_rejected_before_any_repository_or_runtime_port() {
    for attachment in [None, Some(AgentAttachmentId::new())] {
        let mut command = normal_stateless_command();
        command.kind = RunKind::Invocation;
        command.attachment_id = attachment;
        command.requires_state = true;
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(AutoExitProvider::new(Arc::clone(&log)));
        let orchestrator = RunOrchestrator::new(
            Arc::new(repository::NoIoRepository),
            Arc::new(MemoryVolumeStore::new(
                command.instance_id,
                Arc::clone(&log),
            )),
            provider.clone(),
            Arc::new(repository::NoIoSpecFactory),
            32 * 1024 * 1024,
        )
        .with_workspace_manager(Arc::new(RecordingWorkspaceManager {
            log: Arc::clone(&log),
        }))
        .with_runtime_git_workspace_manager(Arc::new(RecordingRuntimeGitWorkspaceManager {
            log: Arc::clone(&log),
        }))
        .with_runtime_manager(Arc::new(RecordingRuntimeManager {
            log: Arc::clone(&log),
        }))
        .with_launch_authorizer(Arc::new(RecordingLaunchAuthorizer {
            log: Arc::clone(&log),
        }))
        .with_resource_observer(Arc::new(RecordingResourceObserver {
            log: Arc::clone(&log),
        }))
        .with_authority_manager(Arc::new(RecordingAuthorityManager {
            log: Arc::clone(&log),
            reject_acknowledgement: false,
        }));
        let result = orchestrator.start_run(&command).await;
        assert!(matches!(
            result,
            Err(OrchestratorError::InvocationUnsupported)
        ));
        assert!(lock(&log).is_empty(), "Invocation reached a runtime port");
        assert!(provider.spec().is_none());
        // Rejection precedes even invalid configured complete-set mode checks.
        let result = orchestrator
            .with_volume_preparation()
            .start_run(&command)
            .await;
        assert!(matches!(
            result,
            Err(OrchestratorError::InvocationUnsupported)
        ));
        assert!(lock(&log).is_empty());
    }
}

use runtime_types::RunId;
use serde_json::json;
use time::OffsetDateTime;
use vm_trait::{VmError, VmEvent, VmExit};
use volume_trait::VolumeError;
use workspace_domain::WorkspaceError;

use super::{
    authority::RunAuthorityError,
    secrets::{RunCompletionError, RunRuntimeError, RunSecretError},
};
use crate::{RepositoryError, StoredVmEvent};

pub(super) fn stored_event(event: VmEvent) -> StoredVmEvent {
    let (event_type, payload) = match event {
        VmEvent::Started { ingress } => (
            "vm.started",
            json!({"ingress": ingress.into_iter().map(|forward| json!({
                "protocol": format!("{:?}", forward.protocol),
                "bind_addr": forward.bind_addr,
                "host_port": forward.host_port,
                "guest_port": forward.guest_port
            })).collect::<Vec<_>>() }),
        ),
        VmEvent::Ready => ("vm.ready", json!({})),
        VmEvent::RuntimeAuthorityAcknowledged {
            session_id,
            generation,
        } => (
            "vm.runtime_authority_acknowledged",
            json!({"session_id": session_id, "generation": generation}),
        ),
        VmEvent::Log { stream, bytes } => (
            "vm.log",
            json!({"stream": format!("{stream:?}"), "bytes": bytes}),
        ),
        VmEvent::Metric(metric) => (
            "vm.metric",
            json!({"name": metric.name, "value": metric.value, "labels": metric.labels}),
        ),
        VmEvent::FinalizeResult { message } => ("vm.finalize_result", json!({"message": message})),
        VmEvent::Exited(exit) => (
            "vm.exited",
            json!({"code": exit.code, "signal": exit.signal}),
        ),
        _ => ("vm.unknown", json!({})),
    };
    StoredVmEvent {
        event_type: event_type.to_owned(),
        payload,
        occurred_at: OffsetDateTime::now_utc(),
    }
}

/// Durable orchestration failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OrchestratorError {
    /// A duplicate delivery observed work that still owns or may own runtime
    /// resources and must be retried after reconciliation.
    #[error("run {0} is still in progress")]
    RunInProgress(RunId),
    /// Run persistence failed.
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    /// Persistent-volume operation failed.
    #[error(transparent)]
    Volume(#[from] VolumeError),
    /// VM lifecycle operation failed.
    #[error(transparent)]
    Vm(#[from] VmError),
    /// Repository workspace or result publication failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// Exact release-artifact or host-context lifecycle failed.
    #[error(transparent)]
    Runtime(#[from] RunRuntimeError),
    /// Exact secret dispatch or ephemeral cleanup failed.
    #[error(transparent)]
    Secret(#[from] RunSecretError),
    /// Generic runtime-authority issuance or cleanup failed.
    #[error(transparent)]
    Authority(#[from] RunAuthorityError),
    /// Post-cleanup domain result processing failed.
    #[error(transparent)]
    Completion(#[from] RunCompletionError),
}

pub(super) struct GuestCompletion {
    pub(super) exit: VmExit,
    pub(super) finalize_message: Option<String>,
}

pub(super) fn capture_finalize(event: &VmEvent, message: &mut Option<String>) {
    if let VmEvent::FinalizeResult { message: finalized } = event {
        *message = Some(finalized.clone());
    }
}

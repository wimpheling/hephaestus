//! Controlled Core boundary tests, not worker/119/native runtime verification.

#[path = "invocation_test_ports.rs"]
mod ports;

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use run_domain::{Run, RunKind, RunState};
use run_orchestrator::RunSecretManager;
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId, ReleaseId, RunId,
};
use time::OffsetDateTime;
use uuid::Uuid;

use ports::{DefaultMetadata, QualifiedMetadata, manager};

// These controlled port futures complete immediately; no runtime/dependency or
// scheduler behavior is being tested. Pending would be a fixture defect.
fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("controlled secret port unexpectedly suspended"),
    }
}

fn invocation() -> Run {
    let id = RunId::new();
    Run {
        id,
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: None,
        kind: RunKind::Invocation,
        requires_state: true,
        command_id: CommandId::new(),
        volume_id: None,
        lease_id: None,
        lease_fencing_token: None,
        vm_id: Some(id.to_string()),
        state: RunState::Provisioning,
        outcome: None,
        exit: None,
        failure: None,
        cancel_requested_at: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
        state_version: 2,
    }
}

#[test]
fn default_invocation_denies_without_historical_identity_or_io() {
    let manager = manager(DefaultMetadata);
    let run = invocation();
    assert!(ready(manager.prepare(&run)).is_err());
    assert!(ready(manager.reauthorize(&run)).is_err());
}

#[test]
fn explicit_controlled_saved_empty_verification_returns_no_mounts() {
    let run = invocation();
    let calls = Arc::new(AtomicUsize::new(0));
    let manager = manager(QualifiedMetadata {
        saved: run.clone(),
        saved_bindings: Vec::new(),
        calls: Arc::clone(&calls),
    });
    assert!(ready(manager.prepare(&run)).unwrap().mounts.is_empty());
    ready(manager.reauthorize(&run)).unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "monitor invokes fresh verifier too"
    );
}

#[test]
fn nonempty_saved_set_never_falls_back_to_historical_dispatch() {
    let run = invocation();
    let calls = Arc::new(AtomicUsize::new(0));
    let manager = manager(QualifiedMetadata {
        saved: run.clone(),
        saved_bindings: vec![Uuid::new_v4()],
        calls: Arc::clone(&calls),
    });
    assert!(ready(manager.prepare(&run)).is_err());
    assert!(ready(manager.reauthorize(&run)).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn kind_or_empty_claim_cannot_replace_exact_saved_revision() {
    let saved = invocation();
    let mut substituted = saved.clone();
    substituted.instance_revision_id = AgentInstanceRevisionId::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let manager = manager(QualifiedMetadata {
        saved,
        saved_bindings: Vec::new(),
        calls: Arc::clone(&calls),
    });
    assert!(ready(manager.prepare(&substituted)).is_err());
    assert!(ready(manager.reauthorize(&substituted)).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

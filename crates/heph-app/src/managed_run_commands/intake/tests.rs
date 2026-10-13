//! Controlled actual intake/owning-task protocol; no VM or DB cleanup proof.
use super::{Operations, Outcome, Request};
use crate::managed_run_commands::{held, ownership};
use heph_run::{CancelRun, RunKind, StartRun};
use run_orchestrator::SupervisorReconciliationCause::{Restart, Stop};
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId, ReleaseId, RunId,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

fn start() -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: None,
        kind: RunKind::Invocation,
        requires_state: false,
    }
}

#[tokio::test]
async fn abandoned_start_waiter_retains_exact_request_and_pipeline_until_release() {
    let operations = Arc::new(Operations::default());
    let command = start();
    let (entered, running) = oneshot::channel();
    let (release, held_pipeline) = oneshot::channel();
    let completed = Arc::new(AtomicBool::new(false));
    let actual = Arc::clone(&completed);
    let receiver = ownership::spawn(&operations, Request::Start(command.clone()), async move {
        entered.send(()).unwrap();
        held_pipeline.await.unwrap();
        actual.store(true, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    running.await.unwrap();
    drop(receiver);
    assert!(!operations.idle_commands().unwrap());
    {
        let state = operations.state().unwrap();
        let entry = state.entries.values().next().unwrap();
        assert!(matches!(&entry.request, Request::Start(saved) if saved == &command));
        assert!(!entry.handle.as_ref().unwrap().is_finished());
        drop(state);
    }
    operations.close().unwrap();
    assert!(operations.register(Request::Start(start())).is_err());
    assert!(!completed.load(Ordering::SeqCst));
    release.send(()).unwrap();
    operations.wait_commands().await.unwrap();
    assert!(completed.load(Ordering::SeqCst));
    assert!(operations.idle_commands().unwrap());
    drop(operations);
}

#[tokio::test]
async fn cleanup_after_close_is_owned_even_when_waiter_drops_and_error_is_retained() {
    let operations = Arc::new(Operations::default());
    operations.close().unwrap();
    let command = CancelRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        reason: "actual controlled command".into(),
    };
    let (entered, running) = oneshot::channel();
    let (release, held_pipeline) = oneshot::channel();
    let receiver = ownership::spawn(&operations, Request::Cancel(command.clone()), async move {
        entered.send(()).unwrap();
        held_pipeline.await.unwrap();
        Err::<(), _>(held("original secret cleanup uncertainty"))
    })
    .unwrap();
    running.await.unwrap();
    drop(receiver);
    assert!(!operations.idle_commands().unwrap());
    release.send(()).unwrap();
    operations.wait_commands().await.unwrap();
    let failures = operations.failures().unwrap();
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0]
            .to_string()
            .contains("original secret cleanup uncertainty")
    );
    let state = operations.state().unwrap();
    let entry = state.entries.values().next().unwrap();
    assert!(matches!(&entry.request, Request::Cancel(saved) if saved == &command));
    assert!(matches!(&entry.outcome, Outcome::Failed(saved) if Arc::ptr_eq(saved, &failures[0])));
    drop(state);
    assert!(operations.register(Request::Start(start())).is_err());
    drop(operations);
}

#[test]
fn successful_restart_never_reopens_concurrent_close_and_failed_restart_stays_paused() {
    let operations = Operations::default();
    let first = operations.register(Request::Reconcile(Restart)).unwrap();
    assert!(operations.register(Request::Start(start())).is_err());
    operations
        .finish(first, Some(Arc::new(held("later-family Held"))))
        .unwrap();
    assert!(operations.register(Request::Start(start())).is_err());
    let retry = operations.register(Request::Reconcile(Restart)).unwrap();
    operations.close().unwrap();
    assert!(operations.finish(retry, None).unwrap().is_none());
    assert!(operations.register(Request::Start(start())).is_err());
    let stop = operations.register(Request::Reconcile(Stop)).unwrap();
    assert!(operations.finish(stop, None).unwrap().is_none());
    assert!(operations.register(Request::Start(start())).is_err());
    drop(operations);
}

#[tokio::test]
async fn queued_cleanup_prevents_lifecycle_settlement_without_discarding_original() {
    let operations = Operations::default();
    let lifecycle = operations.register(Request::Reconcile(Restart)).unwrap();
    let actual = CancelRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        reason: "retained actual request".into(),
    };
    let cleanup = operations
        .register(Request::Cancel(actual.clone()))
        .unwrap();
    assert!(operations.finish(lifecycle, None).unwrap().is_some());
    assert!(operations.register(Request::Start(start())).is_err());
    assert!(
        matches!(&operations.state().unwrap().entries[&cleanup].request, Request::Cancel(saved) if saved == &actual)
    );
    operations.attach(cleanup, tokio::spawn(async {})).unwrap();
    operations.finish(cleanup, None).unwrap();
    operations.wait_commands().await.unwrap();
    let retry = operations.register(Request::Reconcile(Restart)).unwrap();
    assert!(operations.finish(retry, None).unwrap().is_none());
    assert!(operations.register(Request::Start(start())).is_ok());
    drop(operations);
}

#[tokio::test]
async fn finish_between_scan_and_idle_still_requires_actual_task_join() {
    let operations = Arc::new(Operations::default());
    let sequence = operations.register(Request::Start(start())).unwrap();
    let (release, task_tail) = oneshot::channel();
    operations
        .attach(
            sequence,
            tokio::spawn(async move {
                task_tail.await.unwrap();
            }),
        )
        .unwrap();
    assert!(operations.settled_handles().unwrap().is_empty());
    // This is the exact former race: outcome publishes after the handle scan.
    operations.finish(sequence, None).unwrap();
    assert!(operations.idle_commands().unwrap());
    assert!(!operations.joined_commands().unwrap());
    let handles = operations.settled_handles().unwrap();
    assert_eq!(handles.len(), 1);
    let (_, handle) = handles.into_iter().next().unwrap();
    assert!(!handle.is_finished());
    assert!(!operations.joined_commands().unwrap());
    release.send(()).unwrap();
    handle.await.unwrap();
    operations.mark_joined(sequence).unwrap();
    assert!(operations.joined_commands().unwrap());
    drop(operations);
}

#[tokio::test]
async fn late_completed_unjoined_command_blocks_settlement_until_actual_join_and_retry() {
    let operations = Arc::new(Operations::default());
    let lifecycle = operations.register(Request::Reconcile(Restart)).unwrap();
    assert!(operations.joined_commands().unwrap());
    let request = CancelRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        reason: "late actual cleanup".into(),
    };
    let reply = ownership::spawn(&operations, Request::Cancel(request.clone()), async {
        Ok(())
    })
    .unwrap();
    reply.await.unwrap().unwrap();
    assert!(operations.idle_commands().unwrap());
    assert!(!operations.joined_commands().unwrap());
    let original = operations.finish(lifecycle, None).unwrap().unwrap();
    assert!(operations.register(Request::Start(start())).is_err());
    {
        let state = operations.state().unwrap();
        assert!(state.entries.values().any(|entry| {
            matches!(&entry.request, Request::Cancel(saved) if saved == &request)
                && entry.handle.is_some()
                && !entry.joined
        }));
        drop(state);
    }
    operations.wait_commands().await.unwrap();
    assert!(operations.joined_commands().unwrap());
    let retry = operations.register(Request::Reconcile(Restart)).unwrap();
    assert!(operations.finish(retry, None).unwrap().is_none());
    assert!(operations.register(Request::Start(start())).is_ok());
    assert!(
        operations
            .failures()
            .unwrap()
            .iter()
            .any(|cause| Arc::ptr_eq(cause, &original))
    );
    drop(operations);
}

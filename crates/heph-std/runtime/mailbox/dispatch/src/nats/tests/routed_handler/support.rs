use crate::{
    MailboxCommandHandler, MailboxDispatchCommand, MailboxDispatchStore, MailboxDispatchStoreError,
    MailboxOutboxRecord,
};
use async_trait::async_trait;
use run_domain::{CancelRun, Run, StartRun};
use run_orchestrator::{OrchestratorError, RepositoryError, RunCommandExecutor};
use std::{
    collections::VecDeque,
    future::{Future, poll_fn},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Call {
    Apply(String, MailboxDispatchCommand),
    Claim(MailboxDispatchCommand),
    Start(StartRun),
    Finished,
}

#[derive(Clone, Copy)]
pub(super) enum StoreFailure {
    None,
    Apply,
    Claim,
}

struct Store {
    calls: Arc<Mutex<Vec<Call>>>,
    claims: Mutex<VecDeque<Option<StartRun>>>,
    failure: StoreFailure,
}

#[async_trait]
impl MailboxDispatchStore for Store {
    async fn apply_command(
        &self,
        subject: &str,
        command: &MailboxDispatchCommand,
    ) -> Result<(), MailboxDispatchStoreError> {
        self.calls
            .lock()
            .expect("calls")
            .push(Call::Apply(subject.to_owned(), command.clone()));
        if matches!(self.failure, StoreFailure::Apply) {
            return Err(MailboxDispatchStoreError("controlled apply failure".into()));
        }
        Ok(())
    }

    async fn claim_dispatch(
        &self,
        command: &MailboxDispatchCommand,
    ) -> Result<Option<StartRun>, MailboxDispatchStoreError> {
        self.calls
            .lock()
            .expect("calls")
            .push(Call::Claim(command.clone()));
        if matches!(self.failure, StoreFailure::Claim) {
            return Err(MailboxDispatchStoreError("controlled claim failure".into()));
        }
        Ok(self
            .claims
            .lock()
            .expect("claims")
            .pop_front()
            .expect("scripted claim"))
    }

    async fn claim_outbox(
        &self,
        _: &[&str],
        _: i64,
    ) -> Result<Vec<MailboxOutboxRecord>, MailboxDispatchStoreError> {
        panic!("handler must not claim publication outbox")
    }
    async fn mark_outbox_published(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<(), MailboxDispatchStoreError> {
        panic!("handler must not mark outbox")
    }
    async fn mark_outbox_failed(
        &self,
        _: Uuid,
        _: Uuid,
        _: &str,
    ) -> Result<(), MailboxDispatchStoreError> {
        panic!("handler must not mark outbox failure")
    }
    async fn record_run_resources(&self, _: &Run) -> Result<(), MailboxDispatchStoreError> {
        panic!("handler must not record resources")
    }
    async fn settle_run(&self, _: &Run) -> Result<(), MailboxDispatchStoreError> {
        panic!("handler must not settle runs")
    }
    async fn recover(&self) -> Result<usize, MailboxDispatchStoreError> {
        panic!("handler must not recover")
    }
    async fn cleanup_expired_payloads(&self, _: i64) -> Result<usize, MailboxDispatchStoreError> {
        panic!("handler must not purge payloads")
    }
}

struct Executor {
    calls: Arc<Mutex<Vec<Call>>>,
    failures: Mutex<usize>,
    pending_once: bool,
}

#[async_trait]
impl RunCommandExecutor for Executor {
    async fn start_run(&self, command: &StartRun) -> Result<(), OrchestratorError> {
        self.calls
            .lock()
            .expect("calls")
            .push(Call::Start(command.clone()));
        let mut pending = self.pending_once;
        poll_fn(|context| {
            if pending {
                pending = false;
                context.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        let fail = {
            let mut failures = self.failures.lock().expect("failures");
            if *failures == 0 {
                false
            } else {
                *failures -= 1;
                true
            }
        };
        if fail {
            return Err(RepositoryError::InvalidData("controlled executor failure").into());
        }
        self.calls.lock().expect("calls").push(Call::Finished);
        Ok(())
    }

    async fn cancel_run(&self, _: &CancelRun) -> Result<bool, OrchestratorError> {
        panic!("mailbox cancellation is a store operation, not run cancellation")
    }
}

pub(super) struct Harness {
    pub(super) handler: MailboxCommandHandler,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl Harness {
    pub(super) fn new(
        claims: Vec<Option<StartRun>>,
        failure: StoreFailure,
        executor_failures: usize,
        pending_once: bool,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let store = Arc::new(Store {
            calls: calls.clone(),
            claims: Mutex::new(claims.into()),
            failure,
        });
        let executor = Arc::new(Executor {
            calls: calls.clone(),
            failures: Mutex::new(executor_failures),
            pending_once,
        });
        Self {
            handler: MailboxCommandHandler::new_routed(store, executor),
            calls,
        }
    }

    pub(super) fn assert_calls(&self, expected: &[Call]) {
        assert_eq!(self.calls.lock().expect("calls").as_slice(), expected);
    }
}

/// Drives only controlled test futures; no runtime, service or timer is involved.
pub(super) fn poll<T>(future: Pin<&mut impl Future<Output = T>>) -> Poll<T> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

pub(super) fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match poll(future.as_mut()) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("controlled future unexpectedly pending"),
    }
}

pub(super) fn start() -> StartRun {
    // A command value, not a fabricated persisted Run or producer receipt.
    serde_json::from_value(serde_json::json!({
        "command_id": Uuid::new_v4(), "run_id": Uuid::new_v4(),
        "instance_id": Uuid::new_v4(), "instance_revision_id": Uuid::new_v4(),
        "release_id": Uuid::new_v4(), "release_agent_id": Uuid::new_v4(),
        "attachment_id": Uuid::new_v4(), "kind": "normal", "requires_state": true,
    }))
    .expect("controlled exact StartRun")
}

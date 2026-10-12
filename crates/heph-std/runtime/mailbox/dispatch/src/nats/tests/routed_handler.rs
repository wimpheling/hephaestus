#[path = "routed_handler/support.rs"]
mod support;

use crate::{
    MAILBOX_CANCEL_SUBJECT, MAILBOX_DISPATCH_SUBJECT, MAILBOX_RECOVERY_SUBJECT,
    MAILBOX_RETRY_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxCommandError, MailboxDispatchCommand,
};
use mailbox_domain::{MailboxEventId, MailboxId, MailboxOperationIdentity};
use run_orchestrator::{OrchestratorError, RepositoryError};
use std::task::Poll;
use support::{Call, Harness, StoreFailure, poll, ready, start};

fn command() -> MailboxDispatchCommand {
    let event_id = MailboxEventId::new();
    MailboxDispatchCommand {
        operation_id: MailboxOperationIdentity::dispatch(MailboxId::new(), event_id, 1).id(),
        event_id,
    }
}

fn applied(command: &MailboxDispatchCommand) -> Call {
    Call::Apply(MAILBOX_DISPATCH_SUBJECT.into(), command.clone())
}

#[test]
fn unknown_subject_never_calls_store_or_executor() {
    let harness = Harness::new(vec![], StoreFailure::None, 0, false);
    let result = ready(harness.handler.handle("unowned.subject", &command()));
    assert!(
        matches!(result, Err(MailboxCommandError::UnknownSubject(subject)) if subject == "unowned.subject")
    );
    harness.assert_calls(&[]);
}

#[test]
fn non_dispatch_subjects_apply_without_start_or_run_cancel() {
    for subject in [
        MAILBOX_WAKE_SUBJECT,
        MAILBOX_RETRY_SUBJECT,
        MAILBOX_CANCEL_SUBJECT,
        MAILBOX_RECOVERY_SUBJECT,
    ] {
        let harness = Harness::new(vec![], StoreFailure::None, 0, false);
        let command = command();
        ready(harness.handler.handle(subject, &command)).expect("apply supported command");
        harness.assert_calls(&[Call::Apply(subject.into(), command)]);
    }
}

#[test]
fn ineligible_claim_succeeds_without_start() {
    let harness = Harness::new(vec![None], StoreFailure::None, 0, false);
    let command = command();
    ready(harness.handler.handle(MAILBOX_DISPATCH_SUBJECT, &command)).expect("ineligible claim");
    harness.assert_calls(&[applied(&command), Call::Claim(command)]);
}

#[test]
fn exact_store_command_starts_once_and_redelivery_reconsults_store() {
    let run = start();
    let harness = Harness::new(vec![Some(run.clone()), None], StoreFailure::None, 0, false);
    let command = command();
    ready(harness.handler.handle(MAILBOX_DISPATCH_SUBJECT, &command)).expect("claimed start");
    ready(harness.handler.handle(MAILBOX_DISPATCH_SUBJECT, &command))
        .expect("controlled completed replay");
    harness.assert_calls(&[
        applied(&command),
        Call::Claim(command.clone()),
        Call::Start(run),
        Call::Finished,
        applied(&command),
        Call::Claim(command),
    ]);
}

#[test]
fn store_failure_propagates_before_any_start() {
    for failure in [StoreFailure::Apply, StoreFailure::Claim] {
        let harness = Harness::new(vec![], failure, 0, false);
        let command = command();
        let result = ready(harness.handler.handle(MAILBOX_DISPATCH_SUBJECT, &command));
        let expected_error = match failure {
            StoreFailure::Apply => "controlled apply failure",
            StoreFailure::Claim => "controlled claim failure",
            StoreFailure::None => unreachable!("failure cases only"),
        };
        assert!(
            matches!(result, Err(MailboxCommandError::Store(error)) if error.0 == expected_error)
        );
        let mut expected = vec![applied(&command)];
        if matches!(failure, StoreFailure::Claim) {
            expected.push(Call::Claim(command));
        }
        harness.assert_calls(&expected);
    }
}

#[test]
fn executor_failure_is_not_success_and_retry_uses_exact_reclaimed_command() {
    let run = start();
    // Controlled store semantics explicitly reoffer the same command. This does
    // not establish PostgreSQL claim eligibility, idempotency or recovery.
    let harness = Harness::new(
        vec![Some(run.clone()), Some(run.clone())],
        StoreFailure::None,
        1,
        false,
    );
    let command = command();
    let result = ready(harness.handler.handle(MAILBOX_DISPATCH_SUBJECT, &command));
    assert!(matches!(
        result,
        Err(MailboxCommandError::Orchestration(
            OrchestratorError::Repository(RepositoryError::InvalidData(
                "controlled executor failure"
            ))
        ))
    ));
    harness.assert_calls(&[
        applied(&command),
        Call::Claim(command.clone()),
        Call::Start(run.clone()),
    ]);
    ready(harness.handler.handle(MAILBOX_DISPATCH_SUBJECT, &command))
        .expect("controlled reclaimed retry");
    harness.assert_calls(&[
        applied(&command),
        Call::Claim(command.clone()),
        Call::Start(run.clone()),
        applied(&command),
        Call::Claim(command),
        Call::Start(run),
        Call::Finished,
    ]);
}

#[test]
fn handler_waits_for_executor_completion() {
    let run = start();
    let harness = Harness::new(vec![Some(run.clone())], StoreFailure::None, 0, true);
    let command = command();
    {
        let mut future = std::pin::pin!(harness.handler.handle(MAILBOX_DISPATCH_SUBJECT, &command));
        assert!(poll(future.as_mut()).is_pending());
        harness.assert_calls(&[
            applied(&command),
            Call::Claim(command.clone()),
            Call::Start(run.clone()),
        ]);
        assert!(matches!(poll(future.as_mut()), Poll::Ready(Ok(()))));
    }
    harness.assert_calls(&[
        applied(&command),
        Call::Claim(command),
        Call::Start(run),
        Call::Finished,
    ]);
}

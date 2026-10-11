use super::InvocationExecutionPhase;
use crate::{OrchestratorError, RunAuthorityError};

fn original_failure() -> OrchestratorError {
    RunAuthorityError::redacted("original observed withdrawal").into()
}

#[tokio::test]
async fn failure_before_actual_completion_retains_the_owned_primary_error() {
    let phase = InvocationExecutionPhase::default();
    phase.fail(original_failure());
    assert!(!phase.complete_wait());
    assert!(!phase.exited());
    phase.failure_signal().await;
    let error = phase.take_failure().unwrap();
    assert!(matches!(error, OrchestratorError::Authority(_)));
    assert!(error.to_string().contains("original observed withdrawal"));
    assert!(phase.take_failure().is_none());
}

#[tokio::test]
async fn successful_completion_before_failure_keeps_post_exit_checks_pending() {
    let phase = InvocationExecutionPhase::default();
    assert!(phase.complete_wait());
    phase.exit_signal().await;
    phase.fail(original_failure());
    assert!(phase.exited());
    assert!(phase.take_failure().is_none());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), phase.failure_signal())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unselected_ready_failure_signal_does_not_consume_the_error() {
    let phase = InvocationExecutionPhase::default();
    phase.fail(original_failure());
    tokio::select! {
        biased;
        () = std::future::ready(()) => {},
        () = phase.failure_signal() => panic!("execution result is selected first"),
    }
    assert!(
        phase
            .take_failure()
            .unwrap()
            .to_string()
            .contains("original observed withdrawal")
    );
}

#[tokio::test]
async fn first_failure_cannot_be_overwritten_or_changed_to_exit() {
    let phase = InvocationExecutionPhase::default();
    phase.fail(original_failure());
    phase.fail(RunAuthorityError::redacted("later cleanup closure").into());
    assert!(!phase.complete_wait());
    assert!(
        phase
            .take_failure()
            .unwrap()
            .to_string()
            .contains("original observed withdrawal")
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), phase.exit_signal())
            .await
            .is_err()
    );
}

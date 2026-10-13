//! Controlled result mapping; native queued zero-history qualification is separate.

use super::{InvocationCancelEvidence, RepositoryError, queued_authority_absence};

#[test]
fn positively_qualified_queued115_allows_absent_authority_cleanup() {
    assert!(queued_authority_absence(Ok(InvocationCancelEvidence::Queued115)).is_ok());
}

#[test]
fn consumed117_missing_session_keeps_the_original_error() {
    let error = queued_authority_absence(Ok(InvocationCancelEvidence::Consumed117)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "run authority operation failed: runtime session is unavailable"
    );
}

#[test]
fn unknown_foreign_or_unsupported_classification_never_allows_absence() {
    for message in [
        "unknown queued authority history",
        "foreign queued cancellation owner",
        "Invocation cancellation classification is unsupported",
        "queued original configuration differs",
        "queued original has an orphan authorization snapshot",
    ] {
        let error =
            queued_authority_absence(Err(RepositoryError::InvalidData(message))).unwrap_err();
        assert!(error.to_string().contains(message));
    }
}

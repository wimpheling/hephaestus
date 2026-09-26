//! Error conversion and stable command names.

use review_domain::ControlKind;
use review_service::{ReviewOutboxStoreError, ReviewRepositoryError};

pub const fn kind_name(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::CancelRun => "cancel_run",
        ControlKind::RetryRun => "retry_run",
        ControlKind::ApproveResult => "approve_result",
        ControlKind::RejectResult => "reject_result",
    }
}

// The error is owned because `sqlx::Error` is returned by `map_err`; converting
// it immediately preserves its provider detail in the port's string error.
#[allow(clippy::needless_pass_by_value)]
pub fn db_error(error: sqlx::Error) -> ReviewRepositoryError {
    infrastructure(error.to_string())
}

pub const fn infrastructure(error: String) -> ReviewRepositoryError {
    ReviewRepositoryError::Infrastructure(error)
}

// See `db_error`: ownership comes from `map_err` and is consumed into text.
#[allow(clippy::needless_pass_by_value)]
pub fn store_error(error: sqlx::Error) -> ReviewOutboxStoreError {
    ReviewOutboxStoreError(error.to_string())
}

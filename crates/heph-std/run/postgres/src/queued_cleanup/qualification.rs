//! Positive prepared queued cleanup hydration; an open candidate is never a `Run`.

use run_domain::{Run, RunCleanupHostId};
use run_orchestrator::{RepositoryError, RunCleanupWakeRequest};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

use crate::{cleanup_wake::qualification_error, model::RunRow};

pub async fn load_prepared(
    tx: &mut Transaction<'_, Postgres>,
    request: &RunCleanupWakeRequest,
    original: Value,
    scope: &RunCleanupHostId,
    config: &[u8],
) -> Result<Run, RepositoryError> {
    let row: RunRow =
        sqlx::query_as("SELECT * FROM load_invocation119_cleanup_only($1,$2,$3,$4,$5,$6)")
            .bind(request.run_id().as_uuid())
            .bind(request.instance_id().as_uuid())
            .bind(original)
            .bind(scope.provider_namespace())
            .bind(scope.host_id())
            .bind(config)
            .fetch_one(&mut **tx)
            .await
            .map_err(qualification_error)?;
    // The prepared facade proves genuine115, original processed Cancel, exact
    // current config/owner, closed zero-set target and phase receipt/completion.
    // Generic TryFrom and consumed117 inspection remain unchanged.
    row.into_checked_invocation()
}

pub async fn lock_prepare(
    tx: &mut Transaction<'_, Postgres>,
    request: &RunCleanupWakeRequest,
    original: Value,
    scope: &RunCleanupHostId,
    config: &[u8],
) -> Result<(), RepositoryError> {
    // All returned early keys were acquired before parents. SQL repeats TRY,
    // parent/late NOWAIT, then freshly requalifies candidate or prepared replay.
    sqlx::query("SELECT lock_prepare_queued_run_cleanup_wake119($1,$2,$3,$4,$5,$6)")
        .bind(request.run_id().as_uuid())
        .bind(request.instance_id().as_uuid())
        .bind(original)
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(config)
        .execute(&mut **tx)
        .await
        .map_err(qualification_error)?;
    Ok(())
}

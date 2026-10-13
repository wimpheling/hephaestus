//! Narrow worker facades compare original immutable lineage in one snapshot.

use run_domain::RunCleanupHostId;
use run_orchestrator::{RepositoryError, RunCleanupWakeRequest, RunCleanupWakeSnapshot};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};

use super::{canonical, qualification_error};
use crate::model::RunRow;

pub(super) fn original(request: &RunCleanupWakeRequest) -> Value {
    let pins = request.original();
    json!({
        "drain_attempt": pins.drain_attempt,
        "drain_command": pins.drain_command,
        "removal_id": pins.removal_id,
        "command_key": pins.command_key,
        "input_hash": pins.input_hash,
        "expected_version": pins.expected_version,
        "creator": pins.creator,
        "request": pins.request,
    })
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    request: &RunCleanupWakeRequest,
    scope: &RunCleanupHostId,
    config: Option<&[u8]>,
) -> Result<RunCleanupWakeSnapshot, RepositoryError> {
    let row: RunRow = sqlx::query_as("SELECT * FROM load_run_cleanup_wake118($1,$2,$3,$4,$5,$6)")
        .bind(request.run_id().as_uuid())
        .bind(request.instance_id().as_uuid())
        .bind(original(request))
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(config)
        .fetch_one(&mut **tx)
        .await
        .map_err(qualification_error)?;
    // Only the facade's immutable117 correlation admits Invocation hydration.
    let run = if row.is_invocation() {
        row.into_checked_invocation()?
    } else {
        row.try_into()?
    };
    let binding = canonical::binding(tx, request, scope, &run).await?;
    Ok(RunCleanupWakeSnapshot {
        request: request.clone(),
        run,
        binding,
    })
}

pub(super) async fn lock(
    tx: &mut Transaction<'_, Postgres>,
    request: &RunCleanupWakeRequest,
    scope: &RunCleanupHostId,
    config: Option<&[u8]>,
) -> Result<(), RepositoryError> {
    sqlx::query("SELECT lock_run_cleanup_wake118($1,$2,$3,$4,$5,$6)")
        .bind(request.run_id().as_uuid())
        .bind(request.instance_id().as_uuid())
        .bind(original(request))
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(config)
        .execute(&mut **tx)
        .await
        .map_err(qualification_error)?;
    Ok(())
}

// Only the strict prepared facade can supply a checked queued cleanup Run.
// Global target selection and the original request comparison reuse this codec.
pub(super) async fn load_queued(
    tx: &mut Transaction<'_, Postgres>,
    request: &RunCleanupWakeRequest,
    scope: &RunCleanupHostId,
    config: &[u8],
) -> Result<RunCleanupWakeSnapshot, RepositoryError> {
    let run =
        crate::queued_cleanup::load_prepared(tx, request, original(request), scope, config).await?;
    let binding = canonical::binding(tx, request, scope, &run).await?;
    Ok(RunCleanupWakeSnapshot {
        request: request.clone(),
        run,
        binding,
    })
}

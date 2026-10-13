//! Explicit queued cleanup selection; kind/consumption labels grant no authority.

mod context;
mod prepared;
mod qualification;

pub use context::Context;
pub use qualification::{load_prepared, lock_prepare};

use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};

use crate::errors::storage;

// The caller already validated actual OwnedCanonical configuration/owner/wake
// opt-in and opened a role-checked transaction, before any metadata query/lock.
// Presence selects the unchanged consumed facade; only its positive proof can
// authorize that branch. No117 must independently prove queued cleanup below.
pub async fn configuration_for_queued<'config>(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
    config: Option<&'config [u8]>,
) -> Result<Option<&'config [u8]>, RepositoryError> {
    let kind: String = sqlx::query_scalar("SELECT run_kind FROM runs WHERE id=$1")
        .bind(run.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage)?
        .ok_or(RepositoryError::NotFound(run))?;
    match kind.as_str() {
        "normal" | "update" => return Ok(None),
        "invocation" => {}
        _ => {
            return Err(RepositoryError::InvalidData(
                "cleanup wake Run kind is unsupported",
            ));
        }
    }
    let consumed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=$1)",
    )
    .bind(run.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)?;
    if consumed {
        return Ok(None);
    }
    Ok(Some(config.ok_or(RepositoryError::InvalidData(
        "qualified Invocation configuration is absent",
    ))?))
}

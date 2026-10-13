//! Explicit119 lifecycle locks. The default117 composition remains queued-only.

use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};

use crate::{PgRunRepository, errors::storage, model::RunRow};

impl PgRunRepository {
    // Default public107 composition must never infer configured119 scope from
    // the saved receipt. Kind is immutable; read failures remain storage errors
    // and absence retains the existing NotFound result before any mutation.
    pub(crate) async fn reject_default_invocation(
        &self,
        run: RunId,
    ) -> Result<(), RepositoryError> {
        if self.execution_profile.is_some() {
            return Ok(());
        }
        let kind: String = sqlx::query_scalar("SELECT run_kind FROM runs WHERE id=$1")
            .bind(run.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run))?;
        if kind == "invocation" {
            return Err(RepositoryError::InvalidData(
                "default composition does not support Invocation lifecycle",
            ));
        }
        Ok(())
    }

    // The caller owns a new transaction. Resolve kind without taking a child
    // lock; for Invocation reserve actual signed early keys before all rows.
    pub(super) async fn invocation_lifecycle(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
    ) -> Result<Option<RunRow>, RepositoryError> {
        let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
        if !worker {
            return Err(RepositoryError::InvalidData(
                "profiled lifecycle requires worker role",
            ));
        }
        let kind: String = sqlx::query_scalar("SELECT run_kind FROM runs WHERE id=$1")
            .bind(run.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run))?;
        if kind != "invocation" {
            return Ok(None);
        }
        if !self.invocation_lease_runtime {
            return Err(RepositoryError::InvalidData(
                "Invocation lifecycle requires explicit119 composition",
            ));
        }
        let config = serde_json::to_vec(&self.invocation_configuration()?.canonical_json())
            .map_err(storage)?;
        crate::invocation_admission::reservations119::acquire(tx, run).await?;
        let row: RunRow =
            sqlx::query_as("SELECT execution.* FROM lock_qualified_invocation119($1,$2) execution")
                .bind(run.as_uuid())
                .bind(config)
                .fetch_one(&mut **tx)
                .await
                .map_err(storage)?;
        // SQL rechecks the complete immutable graph and configured tuple under
        // locks. Typed Run hydration follows at the repository port boundary.
        // Bind/progress guards check live authority; inspection also supports
        // cleanup after source/gate loss without clearing unknown fences.
        Ok(Some(row))
    }
}

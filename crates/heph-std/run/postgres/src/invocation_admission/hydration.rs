use run_domain::Run;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};

use crate::{PgRunRepository, errors::storage, model::RunRow};

impl PgRunRepository {
    /// Inspects immutable consumed producer data without borrowing current rights.
    ///
    /// This worker-only method is data inspection, not execution/cleanup authority.
    /// It requires explicit configuration; it never creates or consumes a Run.
    ///
    /// # Errors
    /// Holds absent/contradictory117 history and denies a nonworker pool.
    pub async fn inspect_consumed_invocation(&self, run: RunId) -> Result<Run, RepositoryError> {
        let result = self.load_guarded_invocation_or_ordinary(run).await?;
        if result.kind != run_domain::RunKind::Invocation {
            return Err(RepositoryError::InvalidData("expected consumed Invocation"));
        }
        Ok(result)
    }

    pub(crate) async fn load_guarded_invocation_or_ordinary(
        &self,
        run: RunId,
    ) -> Result<Run, RepositoryError> {
        let configuration = self.invocation_configuration()?;
        let config = serde_json::to_vec(&configuration.canonical_json()).map_err(storage)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        if role != "hephaestus_worker" {
            return Err(RepositoryError::InvalidData(
                "Invocation inspection requires worker role",
            ));
        }
        let row: RunRow = sqlx::query_as("SELECT * FROM runs WHERE id=$1")
            .bind(run.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run))?;
        let result = if row.is_invocation() {
            require_lineage(&mut tx, run, &config).await?;
            row.into_checked_invocation()?
        } else {
            row.try_into()?
        };
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }

    pub(crate) async fn hydrate_invocation_or_ordinary(
        &self,
        row: RunRow,
    ) -> Result<Run, RepositoryError> {
        if row.is_invocation() {
            return self.inspect_consumed_invocation(row.run_id()).await;
        }
        row.try_into()
    }
}

async fn require_lineage(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
    config: &[u8],
) -> Result<(), RepositoryError> {
    let exact: bool = sqlx::query_scalar("SELECT inspect_qualified_invocation117($1) AND EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=$1 AND config_bytes=$2)")
        .bind(run.as_uuid()).bind(config).fetch_one(&mut **tx).await.map_err(storage)?;
    if !exact {
        return Err(RepositoryError::InvalidData(
            "immutable117 Invocation lineage is absent or conflicting",
        ));
    }
    Ok(())
}

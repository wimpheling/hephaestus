//! Explicit profile lifecycle checks. The public107 branch never calls these.

mod binding;
mod invocation_runtime;
mod recovery;
mod terminal_success;

pub use recovery::{ProfiledRunRecoveryHold, ProfiledRunRecoveryHoldReason};

use run_domain::{FreshRunExecutionProfile, Run, RunState};
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};

use crate::{PgRunRepository, errors::storage, model::RunRow};

impl PgRunRepository {
    pub(crate) async fn locked_profiled_run(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<RunRow, RepositoryError> {
        if let Some(row) = self.invocation_lifecycle(tx, run_id).await? {
            return Ok(row);
        }
        self.check_legacy_placement_lifecycle(tx, run_id).await?;
        let row: RunRow =
            sqlx::query_as("SELECT execution.* FROM profiled_run_locked($1) execution")
                .bind(run_id.as_uuid())
                .fetch_one(&mut **tx)
                .await
                .map_err(storage)?;
        crate::execution_profile::require_original(tx, run_id, self.execution_profile).await?;
        if self.execution_profile == Some(FreshRunExecutionProfile::OwnedCanonical) {
            let scope = self.vm_scope.as_ref().ok_or(RepositoryError::InvalidData(
                "owned lifecycle requires configured provider ownership",
            ))?;
            let matches: bool = sqlx::query_scalar("SELECT COALESCE(vm_provider_namespace=$2 AND vm_host_id=$3 AND vm_binding_kind='planned' AND vm_id=id::text,false) FROM runs WHERE id=$1")
                .bind(run_id.as_uuid()).bind(scope.provider_namespace()).bind(scope.host_id())
                .fetch_one(&mut **tx).await.map_err(storage)?;
            if !matches {
                return Err(RepositoryError::InvalidData(
                    "owned lifecycle scope is unresolved or differs",
                ));
            }
        }
        Ok(row)
    }

    pub(crate) async fn validate_profiled_transition(
        tx: &mut Transaction<'_, Postgres>,
        run_id: RunId,
        next: RunState,
    ) -> Result<(), RepositoryError> {
        sqlx::query("SELECT assert_profiled_run_transition($1,$2)")
            .bind(run_id.as_uuid())
            .bind(crate::model::state_name(next))
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        Ok(())
    }

    pub(crate) async fn profiled_recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        let mut tx = self.profile_inventory_transaction().await?;
        // A scalar profile does not identify a physical provider. Its held
        // inventory is inspectable, but cannot authorize automated destruction.
        if self.execution_profile != Some(FreshRunExecutionProfile::OwnedCanonical) {
            tx.commit().await.map_err(storage)?;
            return Ok(Vec::new());
        }
        let scope = self.vm_scope.as_ref().ok_or(RepositoryError::InvalidData(
            "owned recovery requires configured provider ownership",
        ))?;
        let rows: Vec<RunRow> = sqlx::query_as("SELECT * FROM runs WHERE state<>'cleaned_up' AND execution_profile='owned_canonical' AND vm_provider_namespace=$1 AND vm_host_id=$2 AND vm_binding_kind='planned' AND vm_id=id::text ORDER BY created_at,id")
            .bind(scope.provider_namespace()).bind(scope.host_id()).fetch_all(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        let mut runs = Vec::with_capacity(rows.len());
        for row in rows {
            runs.push(self.hydrate_invocation_or_ordinary(row).await?);
        }
        Ok(runs)
    }
}

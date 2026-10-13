//! Opt-in original-removal cleanup metadata; never execution or provider proof.

mod canonical;
mod qualification;
mod reservations;

use run_domain::{FreshRunExecutionProfile, RunCleanupHostId, RunCleanupVmTargetKind};
use run_orchestrator::{RepositoryError, RunCleanupWakeRequest, RunCleanupWakeSnapshot};
use sqlx::{Postgres, Transaction};

use crate::{PgRunCleanupRepository, PgRunRepository, cleanup::model::Execution, errors::storage};

impl PgRunRepository {
    pub(crate) fn cleanup_wake_configuration(
        &self,
        scope: &RunCleanupHostId,
    ) -> Result<Option<Vec<u8>>, RepositoryError> {
        if self.cleanup_wake_scope.as_ref() != Some(scope)
            || self.vm_scope.as_ref() != Some(scope)
            || self.execution_profile != Some(FreshRunExecutionProfile::OwnedCanonical)
            || self.legacy_vm_scope.is_some()
        {
            // Genuine Legacy113 and pre117 do not obtain substitute lineage.
            return Err(RepositoryError::InvalidData(
                "owning cleanup wake is unsupported for this composition",
            ));
        }
        self.invocation_configuration
            .as_ref()
            .map(|_| {
                serde_json::to_vec(&self.invocation_configuration()?.canonical_json())
                    .map_err(storage)
            })
            .transpose()
    }

    pub(crate) async fn cleanup_wake_transaction(
        &self,
        readonly: bool,
    ) -> Result<Transaction<'_, Postgres>, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let settings = if readonly {
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        } else {
            // Requalification must see commits made while early keys were held.
            sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED READ WRITE")
        };
        settings.execute(&mut *tx).await.map_err(storage)?;
        let role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        if role != "hephaestus_worker" {
            return Err(RepositoryError::InvalidData(
                "cleanup wake requires the configured worker role",
            ));
        }
        Ok(tx)
    }

    pub(crate) async fn read_qualified_cleanup_wake(
        &self,
        request: &RunCleanupWakeRequest,
        scope: &RunCleanupHostId,
    ) -> Result<RunCleanupWakeSnapshot, RepositoryError> {
        if self.cleanup_wake_scope.is_none() {
            return Err(RepositoryError::InvalidData(
                "owning cleanup wake qualification is unsupported",
            ));
        }
        let config = self.cleanup_wake_configuration(scope)?;
        let mut tx = self.cleanup_wake_transaction(true).await?;
        let queued_config = crate::queued_cleanup::configuration_for_queued(
            &mut tx,
            request.run_id(),
            config.as_deref(),
        )
        .await?;
        let snapshot = if let Some(config) = queued_config {
            qualification::load_queued(&mut tx, request, scope, config).await?
        } else {
            qualification::load(&mut tx, request, scope, config.as_deref()).await?
        };
        tx.commit().await.map_err(storage)?;
        Ok(snapshot)
    }

    pub(crate) async fn prepare_qualified_cleanup_wake(
        &self,
        request: &RunCleanupWakeRequest,
        scope: &RunCleanupHostId,
    ) -> Result<RunCleanupWakeSnapshot, RepositoryError> {
        if self.cleanup_wake_scope.is_none() {
            return Err(RepositoryError::InvalidData(
                "owning cleanup wake preparation is unsupported",
            ));
        }
        let config = self.cleanup_wake_configuration(scope)?;
        let mut tx = self.cleanup_wake_transaction(false).await?;
        let queued_config = crate::queued_cleanup::configuration_for_queued(
            &mut tx,
            request.run_id(),
            config.as_deref(),
        )
        .await?;
        // Wait only on complete sorted original reservations, before any row lock.
        if let Some(config) = queued_config {
            reservations::acquire_queued(&mut tx, request, scope, config).await?;
            crate::queued_cleanup::lock_prepare(
                &mut tx,
                request,
                qualification::original(request),
                scope,
                config,
            )
            .await?;
        } else {
            reservations::acquire(&mut tx, request, scope, config.as_deref()).await?;
            qualification::lock(&mut tx, request, scope, config.as_deref()).await?;
        }
        let execution: Execution = sqlx::query_as(
            "SELECT instance_id,instance_revision_id,state,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,volume_acquisition_closed_at FROM runs WHERE id=$1",
        )
        .bind(request.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
        let vm = execution.vm_target()?;
        if vm.kind() != RunCleanupVmTargetKind::Planned
            || vm.host() != Some(scope)
            || vm.vm_id().is_none()
        {
            return Err(RepositoryError::InvalidData(
                "cleanup wake requires positive consumed VM ownership",
            ));
        }
        let cleanup = PgRunCleanupRepository::new(self.pool.clone(), scope.clone());
        cleanup
            .freeze_target_tx(&mut tx, request.run_id(), execution)
            .await?;
        let snapshot = if let Some(config) = queued_config {
            qualification::load_queued(&mut tx, request, scope, config).await?
        } else {
            qualification::load(&mut tx, request, scope, config.as_deref()).await?
        };
        tx.commit().await.map_err(storage)?;
        Ok(snapshot)
    }
}

// Private qualifier denials retain a typed held result; unrelated storage and
// lock failures preserve their original database error and transaction rollback.
pub fn qualification_error(error: sqlx::Error) -> RepositoryError {
    if error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .as_deref()
        == Some("P1180")
    {
        RepositoryError::InvalidData("cleanup wake history is held or contradictory")
    } else {
        storage(error)
    }
}

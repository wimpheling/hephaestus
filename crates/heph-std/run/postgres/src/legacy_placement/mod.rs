//! Explicit Legacy placement metadata; no host IO or canonical receipt creation.

mod create;
mod evidence;
mod inventory;
mod model;

use run_domain::{
    FreshRunExecutionProfile, LegacyVmPlacement, LegacyVmPlacementConsumption,
    LegacyVmPlacementScope, StartRun,
};
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{PgRunRepository, errors::storage};

impl PgRunRepository {
    pub(crate) async fn check_legacy_placement_lifecycle(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<(), RepositoryError> {
        if let Some(scope) = &self.legacy_vm_scope {
            self.require_legacy_scope(scope)?;
            worker(tx).await?;
            sqlx::query("SELECT lock_legacy_vm_placement_scope($1,$2,$3)")
                .bind(run_id.as_uuid())
                .bind(namespace(scope)?)
                .bind(scope.provider_scope().host_id())
                .execute(&mut **tx)
                .await
                .map_err(storage)?;
            let placement = model::load(tx, run_id, scope).await?;
            let state: String = sqlx::query_scalar("SELECT state FROM runs WHERE id=$1")
                .bind(run_id.as_uuid())
                .fetch_one(&mut **tx)
                .await
                .map_err(storage)?;
            if matches!(state.as_str(), "provisioning" | "starting" | "running")
                && placement.consumption() != Some(LegacyVmPlacementConsumption::Provision)
            {
                return Err(RepositoryError::InvalidData(
                    "Legacy IO phase lacks provision-consumed placement",
                ));
            }
        }
        Ok(())
    }

    pub(crate) async fn close_legacy_acquisition(
        &self,
        run_id: RunId,
        scope: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        self.require_legacy_scope(scope)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        worker(&mut tx).await?;
        sqlx::query("SELECT close_legacy_vm_acquisition($1,$2,$3)")
            .bind(run_id.as_uuid())
            .bind(namespace(scope)?)
            .bind(scope.provider_scope().host_id())
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let placement = model::load(&mut tx, run_id, scope).await?;
        tx.commit().await.map_err(storage)?;
        Ok(placement)
    }

    fn require_legacy_scope(&self, scope: &LegacyVmPlacementScope) -> Result<(), RepositoryError> {
        if self.execution_profile != Some(FreshRunExecutionProfile::LegacyScalar)
            || self.legacy_vm_scope.as_ref() != Some(scope)
        {
            return Err(RepositoryError::InvalidData(
                "Legacy placement requires exact configured Legacy scope",
            ));
        }
        Ok(())
    }

    pub(crate) async fn load_legacy_placement(
        &self,
        run_id: RunId,
        scope: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        self.require_legacy_scope(scope)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        worker(&mut tx).await?;
        let placement = model::load(&mut tx, run_id, scope).await?;
        tx.commit().await.map_err(storage)?;
        Ok(placement)
    }

    pub(crate) async fn consume_legacy_placement(
        &self,
        command: &StartRun,
        scope: &LegacyVmPlacementScope,
        kind: LegacyVmPlacementConsumption,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        self.require_legacy_scope(scope)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        worker(&mut tx).await?;
        // The SQL function locks consumer, Run/plan, then globally ordered
        // volume/lease history. It validates the positive birth before CAS.
        sqlx::query("SELECT consume_legacy_vm_placement($1,$2,$3,$4,$5)")
            .bind(command.run_id.as_uuid())
            .bind(command.command_id.as_uuid())
            .bind(namespace(scope)?)
            .bind(scope.provider_scope().host_id())
            .bind(match kind {
                LegacyVmPlacementConsumption::Provision => "provision",
                LegacyVmPlacementConsumption::CleanupOnly => "cleanup_only",
            })
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let placement = model::load(&mut tx, command.run_id, scope).await?;
        if placement.command() != command {
            return Err(RepositoryError::InvalidData(
                "Legacy immutable start command differs",
            ));
        }
        tx.commit().await.map_err(storage)?;
        Ok(placement)
    }
}

async fn worker(tx: &mut Transaction<'_, Postgres>) -> Result<(), RepositoryError> {
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    if role != "hephaestus_worker" {
        return Err(RepositoryError::InvalidData(
            "Legacy placement requires worker role",
        ));
    }
    Ok(())
}

fn namespace(scope: &LegacyVmPlacementScope) -> Result<Uuid, RepositoryError> {
    scope.provider_scope().namespace().parse().map_err(storage)
}

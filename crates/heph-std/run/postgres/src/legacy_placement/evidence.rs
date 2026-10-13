//! Worker-only, scoped live admission and qualified terminal readback.
use crate::{PgRunRepository, errors::storage, model::RunRow};
use run_domain::{LegacyVmPlacement, LegacyVmPlacementConsumption, LegacyVmPlacementScope, Run};
use run_orchestrator::RepositoryError;
use runtime_types::RunId;

impl PgRunRepository {
    pub(crate) async fn assert_legacy_open(
        &self,
        run: RunId,
        scope: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        self.require_legacy_scope(scope)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        super::worker(&mut tx).await?;
        sqlx::query("SELECT lock_legacy_vm_placement_scope($1,$2,$3)")
            .bind(run.as_uuid())
            .bind(super::namespace(scope)?)
            .bind(scope.provider_scope().host_id())
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let placement = super::model::load(&mut tx, run, scope).await?;
        let open:bool=sqlx::query_scalar("SELECT state IN ('queued','leasing_volume','provisioning') AND cancel_requested_at IS NULL AND volume_acquisition_closed_at IS NULL AND profiled_run_pins_match(execution,true) FROM runs execution WHERE id=$1")
            .bind(run.as_uuid()).fetch_one(&mut *tx).await.map_err(storage)?;
        if !open || placement.consumption() != Some(LegacyVmPlacementConsumption::Provision) {
            return Err(RepositoryError::InvalidData(
                "Legacy placement is not open for new IO",
            ));
        }
        tx.commit().await.map_err(storage)?;
        Ok(placement)
    }
    pub(crate) async fn read_legacy_terminal(
        &self,
        run: RunId,
        scope: &LegacyVmPlacementScope,
    ) -> Result<Run, RepositoryError> {
        self.require_legacy_scope(scope)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        super::worker(&mut tx).await?;
        sqlx::query("SELECT lock_legacy_vm_placement_scope($1,$2,$3)")
            .bind(run.as_uuid())
            .bind(super::namespace(scope)?)
            .bind(scope.provider_scope().host_id())
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let placement = super::model::load(&mut tx, run, scope).await?;
        let row:RunRow=sqlx::query_as("SELECT * FROM runs execution WHERE id=$1 AND state='cleaned_up' AND legacy_scalar_terminal_matches(execution) AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.run_id=execution.id AND lease.released_at IS NULL) AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease JOIN run_legacy_vm_placements plan ON plan.run_id=execution.id AND plan.producer_kind='update_hook' AND plan.producer_id=lease.update_id WHERE lease.run_id IS DISTINCT FROM execution.id)")
            .bind(run.as_uuid()).fetch_optional(&mut *tx).await.map_err(storage)?
            .ok_or(RepositoryError::InvalidData("Legacy cleanup has no exact terminal evidence"))?;
        if placement.consumption().is_none() {
            return Err(RepositoryError::InvalidData(
                "Legacy terminal placement is unconsumed",
            ));
        }
        tx.commit().await.map_err(storage)?;
        row.try_into()
    }
}

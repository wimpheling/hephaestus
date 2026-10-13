use run_domain::{
    LegacyVmPlacementHold, LegacyVmPlacementHoldReason, LegacyVmPlacementInventory,
    LegacyVmPlacementScope,
};
use run_orchestrator::RepositoryError;
use uuid::Uuid;

use crate::{PgRunRepository, errors::storage};

#[derive(sqlx::FromRow)]
struct InventoryRow {
    id: Uuid,
    execution_profile: String,
    legacy_vm_placement_required: bool,
    provider_namespace: Option<Uuid>,
    host_id: Option<String>,
    exact: bool,
    closed: bool,
}

impl PgRunRepository {
    pub(crate) async fn inspect_legacy_placements(
        &self,
        scope: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacementInventory, RepositoryError> {
        self.require_legacy_scope(scope)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        super::worker(&mut tx).await?;
        // Inspect global unknown/foreign evidence, including VM-NULL plans and
        // original released tuples. Already terminal positive plans remain
        // visible for qualified callback observers; no automatic action here.
        let rows: Vec<InventoryRow> = sqlx::query_as("SELECT execution.id,execution.execution_profile,execution.legacy_vm_placement_required,plan.provider_namespace,plan.host_id,COALESCE(legacy_vm_placement_matches(execution,plan),false) AS exact,execution.volume_acquisition_closed_at IS NOT NULL AS closed FROM runs execution LEFT JOIN run_legacy_vm_placements plan ON plan.run_id=execution.id WHERE execution.execution_profile IN ('legacy_scalar','historical_unproven') AND (execution.state<>'cleaned_up' OR plan.run_id IS NOT NULL OR EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.run_id=execution.id AND lease.released_at IS NULL)) ORDER BY execution.id")
            .fetch_all(&mut *tx).await.map_err(storage)?;
        let expected = super::namespace(scope)?;
        let mut result = LegacyVmPlacementInventory {
            placements: Vec::new(),
            held: Vec::new(),
            closed_runs: Vec::new(),
        };
        for row in rows {
            let reason = if row.execution_profile == "historical_unproven" {
                Some(LegacyVmPlacementHoldReason::HistoricalUnproven)
            } else if !row.legacy_vm_placement_required || row.provider_namespace.is_none() {
                Some(LegacyVmPlacementHoldReason::MissingBirth)
            } else if row.provider_namespace != Some(expected)
                || row.host_id.as_deref() != Some(scope.provider_scope().host_id())
            {
                Some(LegacyVmPlacementHoldReason::OwnerMismatch)
            } else if !row.exact {
                Some(LegacyVmPlacementHoldReason::CorrelationMismatch)
            } else {
                None
            };
            if let Some(reason) = reason {
                result.held.push(LegacyVmPlacementHold {
                    run_id: row.id.into(),
                    reason,
                });
            } else {
                if row.closed {
                    result.closed_runs.push(row.id.into());
                }
                result
                    .placements
                    .push(super::model::load(&mut tx, row.id.into(), scope).await?);
            }
        }
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
}

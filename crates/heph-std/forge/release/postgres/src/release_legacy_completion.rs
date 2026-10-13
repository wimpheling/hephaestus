//! Read-only scoped Legacy terminal evidence; no physical absence claim.

use run_domain::{LegacyVmPlacementScope, RunKind};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{AgentUpdateId, ReleaseService, ReleaseServiceError, RunId};

impl ReleaseService {
    /// Reads qualified strict Legacy cleanup evidence for an observer callback.
    ///
    /// The trusted Core must first confirm scoped provider cleanup and commit its
    /// qualified terminal transition. This read grants no physical IO authority,
    /// releases no lease and creates no canonical receipt.
    ///
    /// # Errors
    ///
    /// Holds unsupported, foreign, unconsumed or incomplete cleanup histories;
    /// also fails for non-worker callers or storage errors.
    pub async fn legacy_run_cleanup_completed(
        &self,
        run_id: RunId,
    ) -> Result<RunKind, ReleaseServiceError> {
        self.require_legacy_placement_profile()?;
        let scope = self
            .legacy_vm_scope
            .as_ref()
            .ok_or(ReleaseServiceError::InvalidUpdateLifecycle)?;
        let mut tx = self.pool.begin().await?;
        let kind = require_terminal(&mut tx, run_id.as_uuid(), scope).await?;
        tx.commit().await?;
        Ok(kind)
    }

    pub(super) async fn require_legacy_update_cleanup(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        update: AgentUpdateId,
    ) -> Result<(), ReleaseServiceError> {
        if let Some(scope) = &self.legacy_vm_scope {
            require_update_terminal(tx, update, scope).await?;
        }
        Ok(())
    }

    pub(super) async fn require_legacy_normal_drain(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        instance: Uuid,
        gate_closed_at: OffsetDateTime,
    ) -> Result<(), ReleaseServiceError> {
        let Some(scope) = &self.legacy_vm_scope else {
            return Ok(());
        };
        worker(tx).await?;
        // The caller already owns the consumer lock. Inspect every pre-gate
        // Normal run, including mailbox runs and terminal historical rows.
        let runs: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM runs WHERE instance_id=$1 AND run_kind='normal'
             AND created_at <= $2 ORDER BY id FOR UPDATE",
        )
        .bind(instance)
        .bind(gate_closed_at)
        .fetch_all(&mut **tx)
        .await?;
        for run in runs {
            if require_terminal(tx, run, scope).await? != RunKind::Normal {
                return Err(ReleaseServiceError::InvalidUpdateLifecycle);
            }
        }
        Ok(())
    }
}

pub async fn require_update_terminal(
    tx: &mut Transaction<'_, Postgres>,
    update: AgentUpdateId,
    scope: &LegacyVmPlacementScope,
) -> Result<(), ReleaseServiceError> {
    worker(tx).await?;
    // Retry may have cleared hook_run_id. Immutable birth plans retain all
    // original hook identities; a cleared pointer never means no resources.
    let runs: Vec<Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM run_legacy_vm_placements
         WHERE producer_kind='update_hook' AND producer_id=$1 ORDER BY run_id",
    )
    .bind(update.as_uuid())
    .fetch_all(&mut **tx)
    .await?;
    if runs.is_empty() {
        return Err(ReleaseServiceError::InvalidUpdateLifecycle);
    }
    for run in runs {
        if require_terminal(tx, run, scope).await? != RunKind::Update {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
    }
    Ok(())
}

async fn worker(tx: &mut Transaction<'_, Postgres>) -> Result<(), ReleaseServiceError> {
    sqlx::query("SELECT require_run_cleanup_worker()")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn require_terminal(
    tx: &mut Transaction<'_, Postgres>,
    run: Uuid,
    scope: &LegacyVmPlacementScope,
) -> Result<RunKind, ReleaseServiceError> {
    worker(tx).await?;
    let namespace = scope
        .provider_scope()
        .namespace()
        .parse::<Uuid>()
        .map_err(|_| ReleaseServiceError::InvalidUpdateLifecycle)?;
    // Parent -> Run -> immutable plan; no resource-first locks or host-filtered
    // absence. The helper validates all released catalog and producer pins.
    sqlx::query("SELECT lock_legacy_vm_placement_scope($1,$2,$3)")
        .bind(run)
        .bind(namespace)
        .bind(scope.provider_scope().host_id())
        .execute(&mut **tx)
        .await?;
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT execution.run_kind FROM runs execution
         JOIN run_legacy_vm_placements plan ON plan.run_id=execution.id
         WHERE execution.id=$1 AND plan.provider_namespace=$2 AND plan.host_id=$3
           AND execution.state='cleaned_up' AND execution.volume_acquisition_closed_at IS NOT NULL
           AND plan.consumed_at IS NOT NULL AND plan.consumption_kind IN ('provision','cleanup_only')
           AND legacy_vm_placement_matches(execution,plan)
           AND legacy_scalar_terminal_matches(execution)
           AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
               WHERE (lease.run_id=execution.id OR
                      (plan.producer_kind='update_hook' AND lease.update_id=plan.producer_id))
                 AND (lease.released_at IS NULL OR lease.run_id IS DISTINCT FROM execution.id))
           AND ((num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
                 AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id))
             OR (num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=3
                 AND (SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=execution.id)=1
                 AND legacy_scalar_lease_was_released(execution.lease_id,execution.id,
                       execution.volume_id,plan.host_id,execution.lease_fencing_token)))",
    )
    .bind(run)
    .bind(namespace)
    .bind(scope.provider_scope().host_id())
    .fetch_optional(&mut **tx)
    .await?;
    match kind.as_deref() {
        Some("normal") => Ok(RunKind::Normal),
        Some("update") => Ok(RunKind::Update),
        _ => Err(ReleaseServiceError::InvalidUpdateLifecycle),
    }
}

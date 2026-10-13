//! Qualified complete original/lease observation; no preparation or physical proof.
use super::{PgRunCleanupRepository, model::FenceRow};
use crate::{
    errors::storage,
    qualified_invocation_routing::{
        QualifiedInvocationEvidence, QualifiedInvocationRoute, QualifiedInvocationRoutingDecision,
        QualifiedInvocationRoutingHoldReason, snapshot::inspect_in_snapshot,
    },
    queued_cleanup::Context,
};
use run_domain::{RunCleanupLeaseFence, RunCleanupTarget, RunState};
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

#[derive(sqlx::FromRow)]
struct Original {
    id: Uuid,
    any_lease: bool,
    any_target: bool,
}

#[derive(sqlx::FromRow)]
struct Association {
    run_id: Option<Uuid>,
    execution_profile: Option<String>,
    exact: bool,
    stale: bool,
}

impl PgRunCleanupRepository {
    pub(super) async fn inspect_invocation_recovery(
        &self,
        scope: &VmProviderOwnerScope,
        stale_before: Option<OffsetDateTime>,
    ) -> Result<Vec<RunId>, RepositoryError> {
        let config = self.invocation_configuration.as_ref().ok_or(held(
            "qualified Invocation recovery configuration is absent",
        ))?;
        if config.provider_scope() != scope
            || scope.namespace() != self.scope.provider_namespace()
            || scope.host_id() != self.scope.host_id()
            || self
                .queued_invocation_configuration
                .as_ref()
                .is_some_and(|queued| queued.canonical_json() != config.canonical_json())
        {
            return Err(held(
                "qualified Invocation recovery scope/configuration differs",
            ));
        }
        // Isolation precedes every query, including the worker-role observation.
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let result = self.recovery_snapshot(&mut tx, stale_before).await;
        let rollback = tx.rollback().await.map_err(storage);
        match result {
            Err(primary) => {
                drop(rollback);
                Err(primary)
            }
            Ok(ids) => rollback.map(|()| ids),
        }
    }

    async fn recovery_snapshot(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stale_before: Option<OffsetDateTime>,
    ) -> Result<Vec<RunId>, RepositoryError> {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        sqlx::query("SET LOCAL search_path = pg_catalog, public, pg_temp")
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
        if !worker {
            return Err(held("qualified Invocation recovery requires worker role"));
        }
        // No lifecycle/owner filter can hide contradictory canonical originals.
        let originals: Vec<Original> = sqlx::query_as(
            "SELECT execution.id,EXISTS(SELECT 1 FROM public.agent_instance_volume_leases lease WHERE lease.run_id=execution.id) AS any_lease,EXISTS(SELECT 1 FROM public.run_cleanup_targets target WHERE target.run_id=execution.id) AS any_target FROM public.runs execution WHERE execution.execution_profile='owned_canonical' ORDER BY execution.id",
        ).fetch_all(&mut **tx).await.map_err(storage)?;
        let associations = self.recovery_associations(tx, stale_before).await?;
        let config = self.invocation_configuration.as_ref().ok_or(held(
            "qualified Invocation recovery configuration is absent",
        ))?;
        let mut ids = BTreeSet::new();
        let mut observed = BTreeSet::new();
        for original in originals {
            let run = RunId::from_uuid(original.id);
            if !observed.insert(run) || original.id.is_nil() {
                return Err(held("qualified Invocation recovery originals differ"));
            }
            let route = match inspect_in_snapshot(tx, config, run, None).await? {
                QualifiedInvocationRoutingDecision::Qualified(route) => route,
                QualifiedInvocationRoutingDecision::Held(reason) => {
                    return Err(storage(OriginalHold { run, reason }));
                }
            };
            let leases = associations.get(&run).copied();
            if self
                .recovery_candidate(tx, &original, &route, leases, stale_before)
                .await?
            {
                ids.insert(run);
            }
        }
        if associations.keys().any(|run| !observed.contains(run)) {
            return Err(held("canonical recovery lease has no qualified original"));
        }
        Ok(ids.into_iter().collect())
    }

    async fn recovery_associations(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stale_before: Option<OffsetDateTime>,
    ) -> Result<BTreeMap<RunId, bool>, RepositoryError> {
        let rows: Vec<Association> =
            sqlx::query_as(include_str!("invocation_recovery_associations.sql"))
                .bind(stale_before)
                .fetch_all(&mut **tx)
                .await
                .map_err(storage)?;
        let mut runs = BTreeMap::new();
        for row in rows {
            if !row.exact {
                return Err(held(
                    "recovery lease association is unknown or contradictory",
                ));
            }
            if row.execution_profile.as_deref() == Some("owned_canonical") {
                let run = RunId::from_uuid(
                    row.run_id
                        .ok_or(held("canonical recovery lease Run is absent"))?,
                );
                runs.entry(run)
                    .and_modify(|stale| *stale |= row.stale)
                    .or_insert(row.stale);
            }
        }
        Ok(runs)
    }

    async fn recovery_candidate(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        original: &Original,
        route: &QualifiedInvocationRoute,
        leases: Option<bool>,
        stale_before: Option<OffsetDateTime>,
    ) -> Result<bool, RepositoryError> {
        let run = route.original_start().run_id;
        let target = self.load_target(tx, run).await?;
        if original.any_target && target.is_none() {
            return Err(held(
                "Invocation recovery target belongs to a different owner",
            ));
        }
        // Genuine queued open eligibility has made no resource/target history.
        // It remains unconsumed/redeliverable; the projection grants no Start IO.
        if route.run_state() == RunState::Queued
            && !route.cancellation_requested()
            && !route.acquisition_closed()
            && !original.any_lease
            && target.is_none()
        {
            return Ok(false);
        }
        if route.evidence() == QualifiedInvocationEvidence::QueuedIntent115 {
            let config = self.queued_invocation_configuration.as_ref().ok_or(held(
                "queued Invocation recovery prepared facade is unavailable",
            ))?;
            let bytes = serde_json::to_vec(&config.canonical_json()).map_err(storage)?;
            let context = Context::discover(tx, run, &self.scope, &bytes).await?;
            // Discovery positively qualifies the processed candidate or exact
            // prepared replay in this SAME worker RRRO snapshot. An absent
            // target cannot be hydrated; its owning preparation happens later
            // under the live Start claim and per-Run operation mutex.
            if target.is_some() {
                context.load_prepared(tx, run, &self.scope, &bytes).await?;
            }
            if original.any_lease {
                return Err(held("queued Invocation recovery has lease history"));
            }
        }
        if let Some(target) = &target {
            if self.recovery_target(tx, route, target, leases).await? {
                return Ok(false);
            }
        } else if route.run_state() == RunState::CleanedUp {
            return Err(held("cleaned Invocation has no verified cleanup target"));
        }
        Ok(stale_before.is_none()
            || leases == Some(true)
            || route.cancellation_requested()
            || route.acquisition_closed())
    }

    // A completed exact target is an omission only after its genuine receipt,
    // completion and absence of all unreleased leases have been compared.
    async fn recovery_target(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        route: &QualifiedInvocationRoute,
        target: &RunCleanupTarget,
        leases: Option<bool>,
    ) -> Result<bool, RepositoryError> {
        let run = route.original_start().run_id;
        if target.instance_id() != route.original_start().instance_id
            || target.revision_id() != route.original_start().instance_revision_id
            || target.vm_target().host() != Some(&self.scope)
            || target.vm_target().vm_id().map(|id| id.0.as_str()) != Some(run.to_string().as_str())
        {
            return Err(held("Invocation recovery cleanup target differs"));
        }
        let receipt = self.load_receipt(tx, target).await?;
        if route.run_state() == RunState::CleanedUp {
            let completed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.run_cleanup_completions WHERE run_id=$1 AND generation=$2 AND target_digest=$3)")
                .bind(run.as_uuid()).bind(i64::try_from(target.generation()).map_err(storage)?).bind(target.digest().as_slice())
                .fetch_one(&mut **tx).await.map_err(storage)?;
            if leases.is_some() || receipt.is_none() || !completed {
                return Err(held("cleaned Invocation recovery evidence is inconsistent"));
            }
            return Ok(true);
        }
        let rows: Vec<FenceRow> = sqlx::query_as("SELECT run_id,id AS lease_id,volume_id,host_id,fencing_token FROM public.agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL ORDER BY volume_id,id")
            .bind(run.as_uuid()).fetch_all(&mut **tx).await.map_err(storage)?;
        let fences: Vec<RunCleanupLeaseFence> = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;
        if fences.as_slice() != target.leases() {
            return Err(held("Invocation recovery complete lease set differs"));
        }
        Ok(false)
    }
}

const fn held(message: &'static str) -> RepositoryError {
    RepositoryError::InvalidData(message)
}

#[derive(Debug, thiserror::Error)]
#[error("qualified Invocation recovery held Run {run}: {reason:?}")]
struct OriginalHold {
    run: RunId,
    reason: QualifiedInvocationRoutingHoldReason,
}

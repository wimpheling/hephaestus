//! One configured immutable qualifier shared by dispatch and protected preparation.
use crate::{
    PgRunCleanupRepository, PgRunRepository,
    cleanup::model::{Execution, TargetRow},
    errors::storage,
    qualified_invocation_routing::{
        QualifiedInvocationEvidence, QualifiedInvocationRoutingDecision,
        QualifiedInvocationRoutingHoldReason, snapshot::inspect_in_snapshot,
    },
    queued_cleanup::Context,
};
use run_domain::{Run, RunCleanupHostId, RunCleanupTarget};
use run_orchestrator::{
    InvocationCancelEvidence, InvocationCancelPreparation, PreparedInvocationCancellation,
    RepositoryError,
};
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use vm_trait::VmProviderOwnerScope;

impl PgRunRepository {
    pub(crate) async fn classify_invocation_cancel_impl(
        &self,
        run: RunId,
        scope: &VmProviderOwnerScope,
    ) -> Result<InvocationCancelEvidence, RepositoryError> {
        let (host, config) = self.cancellation_configuration(scope)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let result = self
            .cancellation_snapshot(&mut tx, run, &host, &config)
            .await;
        let rollback = tx.rollback().await.map_err(storage);
        match result {
            Err(primary) => {
                drop(rollback);
                Err(primary)
            }
            Ok(evidence) => rollback.map(|()| evidence),
        }
    }

    fn cancellation_configuration(
        &self,
        scope: &VmProviderOwnerScope,
    ) -> Result<(RunCleanupHostId, Vec<u8>), RepositoryError> {
        let host = RunCleanupHostId::new(scope.namespace().into(), scope.host_id().into())
            .map_err(storage)?;
        if !self.invocation_lease_runtime
            || self.invocation_configuration()?.provider_scope() != scope
        {
            return Err(RepositoryError::InvalidData(
                "queued cancellation requires the exact configured Invocation lifecycle",
            ));
        }
        let config =
            self.cleanup_wake_configuration(&host)?
                .ok_or(RepositoryError::InvalidData(
                    "queued cancellation configuration is absent",
                ))?;
        Ok((host, config))
    }

    async fn cancellation_snapshot(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        host: &RunCleanupHostId,
        config: &[u8],
    ) -> Result<InvocationCancelEvidence, RepositoryError> {
        // Isolation/search path precede every read, including role observation.
        // The caller explicitly rolls back both successful and failed snapshots.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        sqlx::query("SET LOCAL search_path=pg_catalog,public,pg_temp")
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
        if !worker {
            return Err(RepositoryError::InvalidData(
                "Invocation cancellation classification requires worker role",
            ));
        }
        let original = inspect_in_snapshot(tx, self.invocation_configuration()?, run, None).await?;
        let route = match original {
            QualifiedInvocationRoutingDecision::Qualified(route) => route,
            QualifiedInvocationRoutingDecision::Held(reason) => {
                return Err(storage(ClassificationHeld { run, reason }));
            }
        };
        match route.evidence() {
            QualifiedInvocationEvidence::QueuedIntent115 => {
                // Discovery positively qualifies processed cancellation and all
                // per-Run zero history without requiring a prepared target.
                let context = Context::discover(tx, run, host, config).await?;
                let target: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM public.run_cleanup_targets WHERE run_id=$1)",
                )
                .bind(run.as_uuid())
                .fetch_one(&mut **tx)
                .await
                .map_err(storage)?;
                if target {
                    context.load_prepared(tx, run, host, config).await?;
                }
                Ok(InvocationCancelEvidence::Queued115)
            }
            QualifiedInvocationEvidence::Consumed117 => {
                // Reuse the complete association predicate, including every
                // unreleased lease for this Run, in the SAME RRRO snapshot.
                let query = format!(
                    "SELECT NOT EXISTS(SELECT 1 FROM ({}) association WHERE association.run_id=$2 AND (NOT association.exact OR association.execution_profile IS DISTINCT FROM 'owned_canonical'))",
                    include_str!("../cleanup/invocation_recovery_associations.sql"),
                );
                let exact: bool = sqlx::query_scalar(&query)
                    .bind(None::<OffsetDateTime>)
                    .bind(run.as_uuid())
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(storage)?;
                if !exact {
                    return Err(RepositoryError::InvalidData(
                        "consumed Invocation cancellation associations differ",
                    ));
                }
                Ok(InvocationCancelEvidence::Consumed117)
            }
        }
    }

    pub(crate) async fn prepare_invocation_cancel_cleanup_impl(
        &self,
        run: RunId,
        scope: &VmProviderOwnerScope,
    ) -> Result<InvocationCancelPreparation, RepositoryError> {
        // Shared positive qualifier: presence alone never selects consumed117.
        if self.classify_invocation_cancel_impl(run, scope).await?
            == InvocationCancelEvidence::Consumed117
        {
            return Ok(InvocationCancelPreparation::Consumed117);
        }
        let (run, target) = self
            .prepare_queued_invocation_cancellation(run, scope)
            .await?;
        Ok(InvocationCancelPreparation::PreparedQueued(Box::new(
            PreparedInvocationCancellation { run, target },
        )))
    }

    pub(crate) async fn prepare_queued_invocation_cancellation(
        &self,
        run: RunId,
        scope: &VmProviderOwnerScope,
    ) -> Result<(Run, RunCleanupTarget), RepositoryError> {
        let (host, config) = self.cancellation_configuration(scope)?;
        let mut tx = self.cleanup_wake_transaction(false).await?;
        let context = Context::discover(&mut tx, run, &host, &config).await?;
        context
            .reserve_and_lock_preparing(&mut tx, run, &host, &config)
            .await?;
        let execution: Execution = sqlx::query_as(
            "SELECT instance_id,instance_revision_id,state,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,volume_acquisition_closed_at FROM runs WHERE id=$1",
        ).bind(run.as_uuid()).fetch_one(&mut *tx).await.map_err(storage)?;
        PgRunCleanupRepository::new(self.pool.clone(), host.clone())
            .freeze_target_tx(&mut tx, run, execution)
            .await?;
        let prepared = context
            .load_prepared(&mut tx, run, &host, &config)
            .await?
            .into_checked_invocation()?;
        let target: TargetRow = sqlx::query_as(
            "SELECT run_id,instance_id,instance_revision_id,generation,vm_target_kind,vm_provider_namespace,vm_host_id,vm_id,target_digest FROM run_cleanup_targets WHERE run_id=$1",
        ).bind(run.as_uuid()).fetch_one(&mut *tx).await.map_err(storage)?;
        // Both genuine native branches prove the full zero set before hydration.
        let target = target.target(Vec::new())?;
        tx.commit().await.map_err(storage)?;
        Ok((prepared, target))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Invocation cancellation held Run {run}: {reason:?}")]
struct ClassificationHeld {
    run: RunId,
    reason: QualifiedInvocationRoutingHoldReason,
}

//! Prepared pre-consumption cleanup owning proof; no execution authority.

use run_domain::{Run, RunCleanupHostId, RunCleanupVmTargetKind};
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};

use super::{PgRunCleanupRepository, model::Execution};
use crate::{errors::storage, queued_cleanup::Context};

pub struct QueuedCleanup {
    context: Context,
    config: Vec<u8>,
}

impl QueuedCleanup {
    pub async fn requalify(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
    ) -> Result<Run, RepositoryError> {
        // Strict proof must succeed before this private checked conversion.
        self.context
            .load_prepared(tx, run, scope, &self.config)
            .await?
            .into_checked_invocation()
    }
}

impl PgRunCleanupRepository {
    // Every caller first opens the existing role-checked worker transaction.
    // Default/public107 performs no kind, consumption or private schema probe.
    pub(super) async fn lock_queued_cleanup(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
    ) -> Result<Option<(QueuedCleanup, Execution)>, RepositoryError> {
        let Some(configuration) = self.queued_invocation_configuration.as_ref() else {
            return Ok(None);
        };
        let config = serde_json::to_vec(&configuration.canonical_json()).map_err(storage)?;
        // Actual kind/117 presence only select ordinary/consumed old behavior;
        // neither labels nor absent consumption establish queued authority.
        if crate::queued_cleanup::configuration_for_queued(tx, run, Some(&config))
            .await?
            .is_none()
        {
            return Ok(None);
        }
        if configuration.provider_scope().namespace() != self.scope.provider_namespace()
            || configuration.provider_scope().host_id() != self.scope.host_id()
        {
            return Err(RepositoryError::InvalidData(
                "Invocation composition differs",
            ));
        }
        let writable_rc: bool = sqlx::query_scalar(
            "SELECT current_setting('transaction_isolation')='read committed' AND current_setting('transaction_read_only')='off'",
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
        if !writable_rc {
            return Err(RepositoryError::InvalidData(
                "queued cleanup requires writable read-committed transaction",
            ));
        }
        let context = Context::discover(tx, run, &self.scope, &config).await?;
        // DATA discovery permits an open candidate. Owning cleanup never does:
        // Wake alone must have closed acquisition and frozen the zero target.
        context.load_prepared(tx, run, &self.scope, &config).await?;
        context
            .reserve_and_lock(tx, run, &self.scope, &config)
            .await?;
        // Fresh strict proof follows all sorted early waits and TRY/NOWAIT locks.
        let prepared = context
            .load_prepared(tx, run, &self.scope, &config)
            .await?
            .into_checked_invocation()?;
        let execution: Execution = sqlx::query_as(
            "SELECT instance_id,instance_revision_id,state,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,volume_acquisition_closed_at FROM runs WHERE id=$1 AND instance_id=$2 AND instance_revision_id=$3 AND vm_provider_namespace=$4 AND vm_host_id=$5",
        )
        .bind(prepared.id.as_uuid())
        .bind(prepared.instance_id.as_uuid())
        .bind(prepared.instance_revision_id.as_uuid())
        .bind(self.scope.provider_namespace())
        .bind(self.scope.host_id())
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
        let vm = execution.vm_target()?;
        if vm.kind() != RunCleanupVmTargetKind::Planned
            || vm.host() != Some(&self.scope)
            || vm.vm_id().is_none()
            || execution.volume_acquisition_closed_at.is_none()
        {
            return Err(RepositoryError::InvalidData(
                "queued cleanup requires closed immutable planned VM ownership",
            ));
        }
        Ok(Some((QueuedCleanup { context, config }, execution)))
    }
}

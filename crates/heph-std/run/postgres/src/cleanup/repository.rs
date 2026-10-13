use async_trait::async_trait;
use run_domain::{Run, RunCleanupHostId, RunCleanupReceipt, RunCleanupTarget};
use run_orchestrator::{RepositoryError, RunCleanupRepository};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, RunId};
use sqlx::{PgPool, Postgres, Transaction};
use vm_trait::VmId;

use crate::errors::storage;

/// Canonical cleanup adapter restricted to a configured provider's worker pool.
///
/// Configuration must supply a persistent owner namespace bound to the actual
/// provider root/account. An actor or the current process cannot invent it.
/// Public receipt values do not establish physical destruction: trusted worker
/// code records them only after the provider confirms exact destruction/absence.
#[derive(Clone)]
pub struct PgRunCleanupRepository {
    pub(super) pool: PgPool,
    pub(super) scope: RunCleanupHostId,
    pub(super) invocation_configuration: Option<release_service::InstanceExecutionConfiguration>,
    pub(super) queued_invocation_configuration:
        Option<release_service::InstanceExecutionConfiguration>,
}

impl PgRunCleanupRepository {
    /// Creates a worker adapter with its exact persistent provider ownership.
    #[must_use]
    pub const fn new(pool: PgPool, scope: RunCleanupHostId) -> Self {
        Self {
            pool,
            scope,
            invocation_configuration: None,
            queued_invocation_configuration: None,
        }
    }

    /// Configures immutable `Invocation` hydration after exact receipt verification.
    ///
    /// Defaults off. Supply the actual trusted provider and volume configuration;
    /// finish requires matching owner scope and immutable115/117 configuration.
    /// This metadata option grants no physical provider or execution authority.
    #[must_use]
    pub fn with_qualified_invocation_configuration(
        mut self,
        configuration: release_service::InstanceExecutionConfiguration,
    ) -> Self {
        self.invocation_configuration = Some(configuration);
        self
    }

    /// Enables only prepared pre-consumption `Invocation` cleanup metadata.
    ///
    /// Defaults off and leaves the consumed configuration option independent.
    /// Supply the actual checked provider and volume configuration for this scope;
    /// every owning port requires genuine115, original processed Cancel, closed
    /// zero-target cleanup proof and exact full configuration. This option grants
    /// no execution or physical provider authority; a real receipt remains required.
    #[must_use]
    pub fn with_qualified_queued_invocation_cleanup(
        mut self,
        configuration: release_service::InstanceExecutionConfiguration,
    ) -> Self {
        self.queued_invocation_configuration = Some(configuration);
        self
    }

    pub(super) async fn worker_transaction(
        &self,
    ) -> Result<Transaction<'_, Postgres>, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        if role != "hephaestus_worker" {
            return Err(RepositoryError::InvalidData(
                "cleanup requires the configured worker role",
            ));
        }
        Ok(tx)
    }

    /// Loads a previously persisted observation before repeating provider IO.
    ///
    /// # Errors
    ///
    /// Rejects actor pools, another configured provider scope, or malformed data.
    pub(super) async fn read_recorded_receipt(
        &self,
        run_id: RunId,
    ) -> Result<Option<RunCleanupReceipt>, RepositoryError> {
        let mut tx = self.worker_transaction().await?;
        let queued = self.lock_queued_cleanup(&mut tx, run_id).await?;
        if queued.is_none() {
            self.lock_execution(&mut tx, run_id).await?;
        }
        let Some(target) = self.load_target(&mut tx, run_id).await? else {
            if queued.is_some() {
                return Err(RepositoryError::InvalidData(
                    "prepared queued cleanup target is unavailable",
                ));
            }
            tx.commit().await.map_err(storage)?;
            return Ok(None);
        };
        let receipt = self.load_receipt(&mut tx, &target).await?;
        if let Some((queued, _)) = queued {
            queued.requalify(&mut tx, run_id, &self.scope).await?;
        }
        tx.commit().await.map_err(storage)?;
        Ok(receipt)
    }
}

#[async_trait]
impl RunCleanupRepository for PgRunCleanupRepository {
    async fn qualified_invocation_recovery_inventory(
        &self,
        scope: &vm_trait::VmProviderOwnerScope,
        stale_before: Option<time::OffsetDateTime>,
    ) -> Result<Vec<RunId>, RepositoryError> {
        self.inspect_invocation_recovery(scope, stale_before).await
    }

    async fn bind_vm_before_provision(
        &self,
        run_id: RunId,
        instance_id: AgentInstanceId,
        revision_id: AgentInstanceRevisionId,
        host: &RunCleanupHostId,
        vm_id: &VmId,
    ) -> Result<(), RepositoryError> {
        self.bind_vm(run_id, instance_id, revision_id, host, vm_id)
            .await
    }
    async fn begin_cleanup(&self, run_id: RunId) -> Result<RunCleanupTarget, RepositoryError> {
        self.freeze_target(run_id).await
    }
    async fn recorded_cleanup_receipt(
        &self,
        run_id: RunId,
    ) -> Result<Option<RunCleanupReceipt>, RepositoryError> {
        self.read_recorded_receipt(run_id).await
    }
    async fn record_cleanup_receipt(
        &self,
        receipt: &RunCleanupReceipt,
    ) -> Result<(), RepositoryError> {
        self.record_receipt(receipt).await
    }
    async fn finish_cleanup(&self, receipt: &RunCleanupReceipt) -> Result<Run, RepositoryError> {
        self.finish(receipt).await
    }
}

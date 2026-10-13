//! Controlled inventory boundary; it confers no SQL or physical authority.
use super::super::super::support::lock;
use super::super::repository::CleanupRepository;
use async_trait::async_trait;
use run_domain::{Run, RunCleanupHostId, RunCleanupReceipt, RunCleanupTarget};
use run_orchestrator::{RepositoryError, RunCleanupRepository};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, RunId};
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;
use vm_trait::{VmId, VmProviderOwnerScope};

pub struct Inventory {
    pub inner: Arc<CleanupRepository>,
    pub scope: VmProviderOwnerScope,
    pub ids: Vec<RunId>,
    pub cutoffs: Mutex<Vec<Option<OffsetDateTime>>>,
}

#[async_trait]
impl RunCleanupRepository for Inventory {
    async fn qualified_invocation_recovery_inventory(
        &self,
        scope: &VmProviderOwnerScope,
        cutoff: Option<OffsetDateTime>,
    ) -> Result<Vec<RunId>, RepositoryError> {
        if scope != &self.scope {
            return Err(RepositoryError::InvalidData(
                "controlled inventory owner differs",
            ));
        }
        lock(&self.cutoffs).push(cutoff);
        Ok(self.ids.clone())
    }
    async fn bind_vm_before_provision(
        &self,
        run: RunId,
        instance: AgentInstanceId,
        revision: AgentInstanceRevisionId,
        host: &RunCleanupHostId,
        vm: &VmId,
    ) -> Result<(), RepositoryError> {
        self.inner
            .bind_vm_before_provision(run, instance, revision, host, vm)
            .await
    }
    async fn begin_cleanup(&self, run: RunId) -> Result<RunCleanupTarget, RepositoryError> {
        self.inner.begin_cleanup(run).await
    }
    async fn recorded_cleanup_receipt(
        &self,
        run: RunId,
    ) -> Result<Option<RunCleanupReceipt>, RepositoryError> {
        self.inner.recorded_cleanup_receipt(run).await
    }
    async fn record_cleanup_receipt(
        &self,
        receipt: &RunCleanupReceipt,
    ) -> Result<(), RepositoryError> {
        self.inner.record_cleanup_receipt(receipt).await
    }
    async fn finish_cleanup(&self, receipt: &RunCleanupReceipt) -> Result<Run, RepositoryError> {
        self.inner.finish_cleanup(receipt).await
    }
}

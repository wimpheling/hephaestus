//! Actual PG cleanup ports, with a completed-begin signal for Core mutex exclusion.
use async_trait::async_trait;
use run_domain::{Run, RunCleanupHostId, RunCleanupReceipt, RunCleanupTarget};
use run_orchestrator::{RepositoryError, RunCleanupRepository};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, RunId};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use vm_trait::VmId;

pub struct Forward {
    pub inner: run_postgres::PgRunCleanupRepository,
    begins: AtomicUsize,
    entered: Notify,
}
impl Forward {
    pub fn new(inner: run_postgres::PgRunCleanupRepository) -> Arc<Self> {
        Arc::new(Self {
            inner,
            begins: AtomicUsize::new(0),
            entered: Notify::new(),
        })
    }
    pub async fn wait_begins(&self, count: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let notification = self.entered.notified();
                if self.begins.load(Ordering::SeqCst) >= count {
                    break;
                }
                notification.await;
            }
        })
        .await
        .unwrap();
    }
}
#[async_trait]
impl RunCleanupRepository for Forward {
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
        let result = self.inner.begin_cleanup(run).await?;
        self.begins.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_waiters();
        Ok(result)
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

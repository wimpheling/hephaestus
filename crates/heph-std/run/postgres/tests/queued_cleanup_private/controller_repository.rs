//! Forwarded real PG ports; controlled Start gate and fixture-only recovery inventory.
use async_trait::async_trait;
use run_domain::{CancelRun, Run, RunCleanupHostId, RunState, StartRun};
use run_orchestrator::{
    CreateRunResult, RepositoryError, RunCleanupWakeRequest, RunCleanupWakeSnapshot, RunRepository,
    StoredVmEvent,
};
use runtime_types::{LeaseId, RunId, VolumeId};
use std::sync::Arc;
use tokio::sync::{Notify, Semaphore};
use vm_trait::{VmExit, VmId};

pub struct Gate {
    pub entered: Notify,
    pub release: Semaphore,
}
impl Gate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            release: Semaphore::new(0),
        })
    }
    pub async fn wait(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.entered.notified())
            .await
            .unwrap();
    }
}
pub struct Forward {
    pub inner: run_postgres::PgRunRepository,
    pub start: Option<Arc<Gate>>,
    pub recovery: Option<RunId>,
}
#[async_trait]
impl RunRepository for Forward {
    async fn prepare_cleanup_wake(
        &self,
        request: &RunCleanupWakeRequest,
        scope: &RunCleanupHostId,
    ) -> Result<RunCleanupWakeSnapshot, RepositoryError> {
        self.inner.prepare_cleanup_wake(request, scope).await
    }
    async fn load_cleanup_wake(
        &self,
        request: &RunCleanupWakeRequest,
        scope: &RunCleanupHostId,
    ) -> Result<RunCleanupWakeSnapshot, RepositoryError> {
        self.inner.load_cleanup_wake(request, scope).await
    }
    async fn create_run(&self, command: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        self.inner.create_run(command).await
    }
    async fn create_run_with_vm_plan(
        &self,
        command: &StartRun,
        scope: &RunCleanupHostId,
        vm: &VmId,
    ) -> Result<CreateRunResult, RepositoryError> {
        if let Some(gate) = &self.start {
            gate.entered.notify_one();
            gate.release.acquire().await.unwrap().forget();
        }
        self.inner.create_run_with_vm_plan(command, scope, vm).await
    }
    async fn ensure_runtime_git_provenance(&self, run: &Run) -> Result<(), RepositoryError> {
        self.inner.ensure_runtime_git_provenance(run).await
    }
    async fn get(&self, run: RunId) -> Result<Run, RepositoryError> {
        self.inner.get(run).await
    }
    async fn bind_resources(
        &self,
        run: RunId,
        volume: Option<VolumeId>,
        lease: Option<LeaseId>,
        fence: Option<i64>,
        vm: &str,
    ) -> Result<Run, RepositoryError> {
        self.inner
            .bind_resources(run, volume, lease, fence, vm)
            .await
    }
    async fn transition(
        &self,
        run: RunId,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        self.inner.transition(run, next, exit, failure).await
    }
    async fn append_vm_event(
        &self,
        run: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError> {
        self.inner.append_vm_event(run, event).await
    }
    async fn request_cancel(&self, command: &CancelRun) -> Result<bool, RepositoryError> {
        self.inner.request_cancel(command).await
    }
    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        if let Some(run) = self.recovery {
            // Scope the controlled fixture inventory using an actual strict PG get.
            // This is Core mutex evidence, not production inventory acceptance.
            Ok(vec![self.inner.get(run).await?])
        } else {
            self.inner.recoverable_runs().await
        }
    }
}

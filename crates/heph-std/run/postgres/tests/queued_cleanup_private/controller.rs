//! Actual Core owning callbacks and PG receipts; provider response is controlled.
use super::queued_fixture::Queued;
use async_trait::async_trait;
use run_orchestrator::{
    RunCleanupRepository, RunCompletionError, RunCompletionObserver, RunOrchestrator,
    RunRepository, VmSpecFactory,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, Semaphore};
use vm_trait::{VmError, VmId, VmInstance, VmProvider, VmProviderOwnerScope, VmSpec};

pub struct Control {
    pub fail: AtomicBool,
    pub pause: AtomicBool,
    pub entered: Notify,
    pub release: Semaphore,
    pub log: Mutex<Vec<&'static str>>,
}
impl Control {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            fail: AtomicBool::new(false),
            pause: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
            log: Mutex::new(Vec::new()),
        })
    }
    pub fn count(&self, event: &str) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|value| **value == event)
            .count()
    }
    pub async fn wait_entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.entered.notified())
            .await
            .expect("actual scoped callback entered");
    }
}
struct Provider {
    scope: VmProviderOwnerScope,
    vm: VmId,
    control: Arc<Control>,
}
#[async_trait]
impl VmProvider for Provider {
    fn name(&self) -> &'static str {
        "controlled-queued-cleanup"
    }
    fn owner_scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        Ok(self.scope.clone())
    }
    async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Err(VmError::InvalidState("queued cleanup cannot provision"))
    }
    async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
        Err(VmError::InvalidState("unscoped queued cleanup forbidden"))
    }
    async fn cleanup_orphan_scoped(
        &self,
        scope: &VmProviderOwnerScope,
        vm: &VmId,
    ) -> Result<(), VmError> {
        assert_eq!(scope, &self.scope);
        assert_eq!(vm, &self.vm);
        self.control.log.lock().unwrap().push("scoped-entered");
        self.control.entered.notify_one();
        if self.control.pause.load(Ordering::SeqCst) {
            self.control.release.acquire().await.unwrap().forget();
        }
        if self.control.fail.load(Ordering::SeqCst) {
            self.control.log.lock().unwrap().push("scoped-failed");
            return Err(VmError::InvalidState("controlled scoped failure"));
        }
        self.control.log.lock().unwrap().push("scoped-confirmed");
        Ok(())
    }
}
struct Spec;
#[async_trait]
impl VmSpecFactory for Spec {
    async fn build(&self, _: &run_domain::Run) -> Result<VmSpec, VmError> {
        Err(VmError::InvalidState("queued cleanup cannot build"))
    }
}
struct Completion {
    pool: sqlx::PgPool,
    control: Arc<Control>,
}
#[async_trait]
impl RunCompletionObserver for Completion {
    async fn after_cleanup(&self, run: &run_domain::Run) -> Result<(), RunCompletionError> {
        assert_eq!(run.state, run_domain::RunState::CleanedUp);
        assert_eq!(run.outcome, Some(run_domain::RunOutcome::Cancelled));
        let counts: (i64,i64,i64,i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM run_cleanup_receipts WHERE run_id=$1),(SELECT count(*) FROM run_cleanup_completions WHERE run_id=$1),(SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1),(SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='run.cleaned_up')",
        ).bind(run.id.as_uuid()).fetch_one(&self.pool).await.unwrap();
        assert_eq!(
            counts,
            (1, 1, 0, 1),
            "observer follows actual committed zero-set completion"
        );
        self.control.log.lock().unwrap().push("completion");
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunCompletionError> {
        Ok(0)
    }
}
pub fn build(
    h: &Queued,
    control: Arc<Control>,
    repository: Option<Arc<dyn RunRepository>>,
    cleanup: Option<Arc<dyn RunCleanupRepository>>,
) -> Arc<RunOrchestrator> {
    let metadata = Arc::new(h.fixture.volumes());
    let volumes = Arc::new(
        volume_local::LocalVolumeStore::new(
            metadata.clone(),
            volume_local::LocalVolumeConfig {
                volume_root: h.fixture.physical.root.path().into(),
                transient_runtime_roots: Vec::new(),
                host_id: h.fixture.scope.host_id().into(),
                lease_duration: std::time::Duration::from_secs(30),
                mkfs_ext4: "/usr/sbin/mkfs.ext4".into(),
            },
        )
        .unwrap()
        .with_run_metadata(metadata),
    );
    let provider = Arc::new(Provider {
        scope: h.fixture.config.provider_scope().clone(),
        vm: h.fixture.vm.clone(),
        control: control.clone(),
    });
    Arc::new(
        RunOrchestrator::new(
            repository.unwrap_or_else(|| Arc::new(h.repository())),
            volumes.clone(),
            provider,
            Arc::new(Spec),
            16_777_216,
        )
        .with_cleanup_repository(cleanup.unwrap_or_else(|| Arc::new(h.cleanup())), volumes)
        .with_volume_preparation()
        .with_qualified_invocation(h.fixture.config.provider_scope().clone())
        .with_completion_observer(Arc::new(Completion {
            pool: h.fixture.pool.clone(),
            control,
        })),
    )
}

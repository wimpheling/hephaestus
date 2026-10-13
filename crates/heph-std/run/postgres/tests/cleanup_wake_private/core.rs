//! Actual core controller and `PostgreSQL` receipts with a controlled owner response.
use super::{observations::rows, wake_fixture::Wake};
use async_trait::async_trait;
use run_orchestrator::{
    RunCleanupRepository, RunCleanupWakeProgress, RunCompletionError, RunCompletionObserver,
    RunOrchestrator, VmSpecFactory,
};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use vm_trait::{VmError, VmId, VmInstance, VmProvider, VmProviderOwnerScope, VmSpec};

struct Provider {
    scope: VmProviderOwnerScope,
    vm: VmId,
    log: Arc<Mutex<Vec<&'static str>>>,
}
#[async_trait]
impl VmProvider for Provider {
    fn name(&self) -> &'static str {
        "controlled-cleanup-wake"
    }
    fn owner_scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        Ok(self.scope.clone())
    }
    async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Err(VmError::InvalidState("cleanup must not provision"))
    }
    async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
        Err(VmError::InvalidState("unscoped cleanup forbidden"))
    }
    async fn cleanup_orphan_scoped(
        &self,
        scope: &VmProviderOwnerScope,
        vm: &VmId,
    ) -> Result<(), VmError> {
        assert_eq!(scope, &self.scope);
        assert_eq!(vm, &self.vm);
        self.log.lock().unwrap().push("scoped-confirmed");
        Ok(())
    }
}
struct Spec;
#[async_trait]
impl VmSpecFactory for Spec {
    async fn build(&self, _: &run_domain::Run) -> Result<VmSpec, VmError> {
        Err(VmError::InvalidState("cleanup must not build"))
    }
}
struct Completion {
    pool: PgPool,
    log: Arc<Mutex<Vec<&'static str>>>,
}
#[async_trait]
impl RunCompletionObserver for Completion {
    async fn after_cleanup(&self, run: &run_domain::Run) -> Result<(), RunCompletionError> {
        assert_eq!(run.state, run_domain::RunState::CleanedUp);
        let complete: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=$1)",
        )
        .bind(run.id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .unwrap();
        assert!(complete, "callback follows actual committed completion");
        let held:i64=sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL").bind(run.id.as_uuid()).fetch_one(&self.pool).await.unwrap();
        assert_eq!(held, 0);
        self.log.lock().unwrap().push("completion");
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunCompletionError> {
        Ok(0)
    }
}

type Log = Arc<Mutex<Vec<&'static str>>>;
pub fn controller(
    h: &Wake,
) -> (
    Arc<RunOrchestrator>,
    Arc<run_postgres::PgRunCleanupRepository>,
    Log,
) {
    let cleanup = Arc::new(
        run_postgres::PgRunCleanupRepository::new(
            h.fixture.birth.worker.clone(),
            h.fixture.scope.clone(),
        )
        .with_qualified_invocation_configuration(h.fixture.config.clone()),
    );
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
    let log = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(Provider {
        scope: h.fixture.config.provider_scope().clone(),
        vm: h.fixture.vm.clone(),
        log: log.clone(),
    });
    let orchestrator = Arc::new(
        RunOrchestrator::new(
            Arc::new(h.repository()),
            volumes.clone(),
            provider,
            Arc::new(Spec),
            16_777_216,
        )
        .with_cleanup_repository(cleanup.clone(), volumes)
        .with_completion_observer(Arc::new(Completion {
            pool: h.fixture.pool.clone(),
            log: log.clone(),
        })),
    );
    (orchestrator, cleanup, log)
}

async fn assert_finish_configuration_denials(h: &Wake, receipt: &run_domain::RunCleanupReceipt) {
    let before = rows(&h.fixture.pool).await;
    for index in 0..6 {
        let mut cleanup = run_postgres::PgRunCleanupRepository::new(
            h.fixture.birth.worker.clone(),
            h.fixture.scope.clone(),
        );
        if index != 0 {
            let scope = VmProviderOwnerScope::new(
                if index == 1 {
                    "foreign".into()
                } else {
                    h.fixture.scope.provider_namespace().into()
                },
                if index == 2 {
                    "foreign".into()
                } else {
                    h.fixture.scope.host_id().into()
                },
            )
            .unwrap();
            let config = release_service::InstanceExecutionConfiguration::new(
                scope,
                if index == 3 {
                    "/different-root".into()
                } else {
                    h.fixture.config.volume_root().into()
                },
                if index == 4 {
                    volume_domain::VolumeRootNamespaceId::from_uuid(uuid::Uuid::new_v4()).unwrap()
                } else {
                    h.fixture.config.volume_namespace()
                },
                h.fixture.config.platform_policy().clone(),
                if index == 5 {
                    "platform/different".into()
                } else {
                    h.fixture.config.platform_version().into()
                },
            )
            .unwrap();
            cleanup = cleanup.with_qualified_invocation_configuration(config);
        }
        let expected = match index {
            0 => "qualified Invocation configuration is absent",
            1 | 2 => "Invocation composition differs",
            _ => "immutable117 Invocation lineage is absent or conflicting",
        };
        assert!(matches!(cleanup.finish_cleanup(receipt).await,
            Err(run_orchestrator::RepositoryError::InvalidData(message)) if message == expected));
        assert_eq!(
            rows(&h.fixture.pool).await,
            before,
            "finish configuration {index}"
        );
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn actual_core_receipt_full_release_completion_and_replay_preserve_newer_generation(
    pool: PgPool,
) {
    let h = Wake::new(&pool, 2).await;
    let before = h.prepare().await;
    super::cancel_evidence::assert_negatives(&h).await;
    let (orchestrator, cleanup, log) = controller(&h);
    assert_eq!(
        orchestrator.clone().wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(*log.lock().unwrap(), vec!["scoped-confirmed", "completion"]);
    let terminal = h.read().await;
    assert_eq!(terminal.binding, before.binding);
    assert_eq!(terminal.run.state, run_domain::RunState::CleanedUp);
    let receipt = cleanup
        .recorded_cleanup_receipt(h.request.run_id())
        .await
        .unwrap()
        .unwrap();
    let original = rows(&pool).await;
    assert_eq!(
        original["run_cleanup_receipts"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        original["run_cleanup_completions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        original["agent_instance_volume_leases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| !row["released_at"].is_null())
    );
    let newer = super::successor::acquire_newer(&h.fixture).await;
    let newer_before = super::successor::holder(&pool, newer).await;
    let stable = rows(&pool).await;
    assert_finish_configuration_denials(&h, &receipt).await;
    assert_eq!(
        cleanup.finish_cleanup(&receipt).await.unwrap(),
        terminal.run
    );
    assert_eq!(rows(&pool).await, stable);
    assert_eq!(h.read().await, terminal);
    assert_eq!(
        orchestrator.wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(super::successor::holder(&pool, newer).await, newer_before);
    assert_eq!(
        rows(&pool).await,
        stable,
        "receipt replay cannot rewrite any current row or event"
    );
    assert_eq!(
        cleanup
            .recorded_cleanup_receipt(h.request.run_id())
            .await
            .unwrap()
            .unwrap(),
        receipt
    );
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|event| **event == "scoped-confirmed")
            .count(),
        1
    );
    h.fixture.physical.assert_unchanged();
    h.fixture.birth.assert_historical_migrations(&pool).await;
}

//! Genuine115 queued birth; actual configured Cancel931f, never positive117.
use crate::fixture::Fixture;
use recipe_application::{PreparedInstanceClosureRepository, PreparedInstanceClosureRequest};
use run_domain::{CancelRun, RunCleanupTarget};
use run_orchestrator::{
    RunCleanupRepository, RunCleanupWakeRequest, RunCleanupWakeSnapshot, RunRepository,
};
use sqlx::PgPool;

pub struct Queued {
    pub fixture: Fixture,
    pub closure: PreparedInstanceClosureRequest,
    pub request: RunCleanupWakeRequest,
    pub cancel: CancelRun,
}
impl Queued {
    pub async fn new(pool: &PgPool, count: usize, delivered: bool) -> Self {
        super::loader::assert_pins();
        let fixture = Fixture::new(pool, count).await;
        super::loader::remaining(pool).await;
        let closure = super::closure::make(&fixture).await;
        super::closure::service(&fixture)
            .close_for_remove(&fixture.birth.actor, &closure)
            .await
            .unwrap();
        let h = Self::from_closed(fixture, closure).await;
        if delivered {
            h.deliver().await;
        }
        h
    }
    pub async fn from_closed(
        mut fixture: Fixture,
        closure: PreparedInstanceClosureRequest,
    ) -> Self {
        let (vm, namespace, host, binding): (String, String, String, String) = sqlx::query_as(
            "SELECT vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind FROM runs WHERE id=$1",
        )
        .bind(fixture.start.run_id.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
        assert_eq!(namespace, fixture.scope.provider_namespace());
        assert_eq!(host, fixture.scope.host_id());
        assert_eq!(binding, "planned");
        // Use the actual immutable stored tuple; no UUID/label fallback.
        fixture.vm = vm_trait::VmId(vm);
        let request = super::closure::committed(&fixture, &closure).await;
        let payload: serde_json::Value = sqlx::query_scalar(
            "SELECT payload FROM outbox WHERE aggregate_id=$1 AND event_type='run.cancel_requested.v1'",
        ).bind(fixture.start.run_id.as_uuid()).fetch_one(&fixture.pool).await.unwrap();
        let cancel: CancelRun = serde_json::from_value(payload).unwrap();
        assert_eq!(cancel.run_id, request.run_id());
        Self {
            fixture,
            closure,
            request,
            cancel,
        }
    }
    pub fn repository(&self) -> run_postgres::PgRunRepository {
        self.fixture
            .repository()
            .with_qualified_cleanup_wake(self.fixture.scope.clone())
    }
    pub fn cleanup(&self) -> run_postgres::PgRunCleanupRepository {
        run_postgres::PgRunCleanupRepository::new(
            self.fixture.birth.worker.clone(),
            self.fixture.scope.clone(),
        )
        .with_qualified_queued_invocation_cleanup(self.fixture.config.clone())
    }
    pub async fn deliver(&self) {
        let before = super::observations::rows(&self.fixture.pool).await;
        assert!(
            self.repository()
                .request_cancel(&self.cancel)
                .await
                .unwrap()
        );
        super::observations::unconsumed(self).await;
        let processed: bool = sqlx::query_scalar(
            "SELECT command_type='cancel_run' AND payload=$2 AND processed_at IS NOT NULL FROM command_inbox WHERE command_id=$1",
        ).bind(self.cancel.command_id.as_uuid()).bind(serde_json::to_value(&self.cancel).unwrap())
        .fetch_one(&self.fixture.pool).await.unwrap();
        assert!(
            processed,
            "actual configured931f writer persisted exact processed Cancel"
        );
        let after = super::observations::rows(&self.fixture.pool).await;
        assert_ne!(after, before);
        assert!(
            !self
                .repository()
                .request_cancel(&self.cancel)
                .await
                .unwrap()
        );
        assert_eq!(super::observations::rows(&self.fixture.pool).await, after);
    }
    pub async fn prepare(&self) -> RunCleanupWakeSnapshot {
        self.repository()
            .prepare_cleanup_wake(&self.request, &self.fixture.scope)
            .await
            .unwrap()
    }
    pub async fn read(&self) -> RunCleanupWakeSnapshot {
        self.repository()
            .load_cleanup_wake(&self.request, &self.fixture.scope)
            .await
            .unwrap()
    }
    pub async fn target(&self) -> RunCleanupTarget {
        self.cleanup()
            .begin_cleanup(self.request.run_id())
            .await
            .unwrap()
    }
}

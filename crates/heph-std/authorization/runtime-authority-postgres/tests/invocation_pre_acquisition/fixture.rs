//! Actual115 birth and117 consumption, retained before any acquisition.

use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use run_domain::{Run, RunState};
use run_orchestrator::RunRepository;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sqlx::PgPool;
use time::OffsetDateTime;

pub struct Fixture {
    pub birth: crate::fixture::Fixture,
    pub repository: PgRuntimeSessionRepository,
    pub run: Run,
}
impl Fixture {
    pub async fn new(pool: &PgPool, count: usize) -> Self {
        super::schema::verify_inputs();
        let birth = Box::pin(crate::fixture::Fixture::new(pool, count)).await;
        super::schema::apply(pool).await;
        Box::pin(Self::consumed(birth)).await
    }
    pub async fn consumed(birth: crate::fixture::Fixture) -> Self {
        birth.consume().await.unwrap();
        let run = birth
            .repository()
            .transition(birth.start.run_id, RunState::LeasingVolume, None, None)
            .await
            .unwrap();
        assert_eq!(run.state, RunState::LeasingVolume);
        assert_eq!(run.vm_id.as_deref(), Some(birth.vm.0.as_str()));
        let repository = PgRuntimeSessionRepository::new(birth.birth.worker.clone())
            .with_qualified_invocation_configuration(birth.config.clone());
        Self {
            birth,
            repository,
            run,
        }
    }
    pub fn config(&self) -> Vec<u8> {
        serde_json::to_vec(&self.birth.config.canonical_json()).unwrap()
    }
    pub async fn raw(
        &self,
        pool: &PgPool,
        run: &Run,
        config: &[u8],
        model: &str,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT verify_invocation119_pre_acquisition($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(run.id.as_uuid())
        .bind(config)
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(run.command_id.as_uuid())
        .bind(run.requires_state)
        .bind(run.vm_id.as_deref())
        .bind(run.state_version)
        .bind(model)
        .fetch_one(pool)
        .await
    }
    pub async fn rows(&self) -> serde_json::Value {
        let graph: serde_json::Value = sqlx::query_scalar(include_str!("rows.sql"))
            .fetch_one(&self.birth.pool)
            .await
            .unwrap();
        serde_json::json!({"prior":self.birth.rows().await,"graph":graph})
    }
    pub async fn allowed(&self) {
        let before = self.rows().await;
        self.repository
            .verify_qualified_invocation_pre_acquisition(&self.run, AUTHORIZATION_MODEL_VERSION)
            .await
            .unwrap();
        assert!(
            self.raw(
                &self.birth.birth.worker,
                &self.run,
                &self.config(),
                AUTHORIZATION_MODEL_VERSION
            )
            .await
            .unwrap()
        );
        assert_eq!(self.rows().await, before);
        self.birth.physical.assert_unchanged();
    }
    pub async fn denied(&self, run: &Run) {
        let before = self.rows().await;
        assert!(
            self.repository
                .verify_qualified_invocation_pre_acquisition(run, AUTHORIZATION_MODEL_VERSION)
                .await
                .is_err()
        );
        assert_eq!(self.rows().await, before);
        self.birth.physical.assert_unchanged();
    }
    pub async fn b2(&self, run: &Run, phase: &str) -> bool {
        sqlx::query_scalar(
            "SELECT verify_invocation119_empty_secrets($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(run.id.as_uuid())
        .bind(self.config())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(run.command_id.as_uuid())
        .bind(run.requires_state)
        .bind(run.vm_id.as_deref())
        .bind(phase)
        .fetch_one(&self.birth.birth.worker)
        .await
        .unwrap()
    }
}
pub fn pg_time() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    OffsetDateTime::from_unix_timestamp_nanos(now.unix_timestamp_nanos().div_euclid(1000) * 1000)
        .unwrap()
}
pub fn code(error: &sqlx::Error) -> Option<std::borrow::Cow<'_, str>> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
}

use async_trait::async_trait;
use capability_domain::GatewayInvocationId;
use capability_domain::{RuntimeCredential, RuntimeCredentialGeneration, RuntimeSessionId};
use runtime_authority::GatewayRuntimeSessionRequest;
use runtime_authority::{RuntimeAuthorityError, RuntimeHandoffStore};
use runtime_authority_postgres::{
    PgGatewayRuntimeAuthorityIssuer, PgGatewayRuntimeSessionRepository,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct CountingHandoff {
    pub creates: Arc<AtomicUsize>,
}

#[async_trait]
impl RuntimeHandoffStore for CountingHandoff {
    fn create(
        &self,
        _session_id: RuntimeSessionId,
        _generation: RuntimeCredentialGeneration,
        _expires_at: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(RuntimeCredential::from_secret([7; 32]))
    }

    fn open(
        &self,
        _session_id: RuntimeSessionId,
        _generation: RuntimeCredentialGeneration,
        _now: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        Ok(RuntimeCredential::from_secret([7; 32]))
    }

    fn destroy(
        &self,
        _session_id: RuntimeSessionId,
        _generation: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeAuthorityError> {
        Ok(())
    }

    fn purge_expired(&self, _now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        Ok(0)
    }
}

pub struct TestContext {
    pub pool: sqlx::PgPool,
    pub issuer: Arc<PgGatewayRuntimeAuthorityIssuer<CountingHandoff>>,
    pub repository: PgGatewayRuntimeSessionRepository,
    pub creates: Arc<AtomicUsize>,
}

pub async fn setup() -> Option<TestContext> {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect runtime authority PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply runtime authority migrations");
    let migration_version: i64 =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE version = 75")
            .fetch_one(&pool)
            .await
            .expect("confirm migration 0075 applied to the connected database");
    assert_eq!(migration_version, 75);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 migration=75");

    let creates = Arc::new(AtomicUsize::new(0));
    let issuer = Arc::new(PgGatewayRuntimeAuthorityIssuer::new(
        pool.clone(),
        CountingHandoff {
            creates: Arc::clone(&creates),
        },
        "test/v1",
    ));
    let repository = PgGatewayRuntimeSessionRepository::new(pool.clone());
    Some(TestContext {
        pool,
        issuer,
        repository,
        creates,
    })
}

pub fn request(
    fixture: &super::fixtures::Fixture,
    issued_at: OffsetDateTime,
    lifetime: time::Duration,
) -> GatewayRuntimeSessionRequest {
    GatewayRuntimeSessionRequest {
        invocation_id: GatewayInvocationId::from_uuid(fixture.invocation),
        gateway_id: fixture.gateway,
        gateway_revision_id: fixture.revision,
        issued_at,
        expires_at: issued_at + lifetime,
    }
}

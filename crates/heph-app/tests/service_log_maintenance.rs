//! Application-level retention acceptance with the optional gateway edge off.

#![cfg(feature = "test-fixtures")]

const TEST_SIGNING_SECRET: &[u8] = b"service-log-retention-test-signing-secret";
const TEST_ROOT_IMAGE: &str = "service-log-retention-test@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
use hephaestus_app::EXPECTED_DATABASE_MIGRATION;
use sqlx::postgres::PgPoolOptions;
use url::Url;
use uuid::Uuid;

pub(crate) struct IsolatedDatabase {
    pub(crate) database_name: String,
    pub(crate) maintenance_url: String,
    pub(crate) target_url: String,
    pub(crate) max_version: i64,
}

impl IsolatedDatabase {
    pub(crate) async fn create(parent_url: &str) -> Self {
        let database_name = format!("hephaestus_service_log_{}", Uuid::new_v4().simple());
        let mut maintenance_url = Url::parse(parent_url).expect("parse PostgreSQL test URL");
        maintenance_url.set_path("/postgres");
        let mut target_url = Url::parse(parent_url).expect("parse PostgreSQL test URL");
        target_url.set_path(&format!("/{database_name}"));

        let maintenance_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(maintenance_url.as_str())
            .await
            .expect("connect PostgreSQL maintenance database");
        sqlx::query(&format!("CREATE DATABASE \"{database_name}\""))
            .execute(&maintenance_pool)
            .await
            .expect("create isolated PostgreSQL database");
        maintenance_pool.close().await;

        let target_pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(target_url.as_str())
            .await
            .expect("connect isolated PostgreSQL database");
        sqlx::migrate!("../../migrations")
            .run(&target_pool)
            .await
            .expect("apply database migrations");
        let max_version: i64 = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT max(version) FROM _sqlx_migrations WHERE success",
        )
        .fetch_one(&target_pool)
        .await
        .expect("read isolated migration marker")
        .expect("isolated migrations exist");
        assert_eq!(max_version, EXPECTED_DATABASE_MIGRATION);
        target_pool.close().await;

        Self {
            database_name,
            maintenance_url: maintenance_url.to_string(),
            target_url: target_url.to_string(),
            max_version,
        }
    }

    pub(crate) async fn drop(self) {
        let maintenance_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.maintenance_url)
            .await
            .expect("reconnect PostgreSQL maintenance database");
        sqlx::query(&format!("DROP DATABASE \"{}\"", self.database_name))
            .execute(&maintenance_pool)
            .await
            .expect("drop isolated PostgreSQL database");
        maintenance_pool.close().await;
    }
}

#[path = "service_log_maintenance/fixture.rs"]
mod fixture;
#[path = "service_log_maintenance/retention.rs"]
mod retention;
#[path = "service_log_maintenance/seed.rs"]
mod seed;

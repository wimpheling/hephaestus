use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::env;

use crate::support::{Fixture, seed_fixture};

#[path = "scenario/capture.rs"]
mod capture;
#[path = "scenario/links.rs"]
mod links;
#[path = "scenario/rls.rs"]
mod rls;

pub struct Context {
    pub admin: PgPool,
    pub worker: PgPool,
    pub fixture: Fixture,
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ui_source_manifest_schema_enforces_capture_and_link_boundaries() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("SKIPPED UI source manifest schema: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::migrate!("../../../../migrations")
        .run(&admin)
        .await
        .expect("apply UI source manifest migration");
    let max_version: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&admin)
            .await
            .expect("read migration marker");
    assert!(max_version >= 82, "migration 0082 must be applied");

    let fixture = seed_fixture(&admin).await;
    let worker = crate::support::worker_pool(&database_url).await;
    let context = Context {
        admin,
        worker,
        fixture,
    };
    capture::run(&context).await;
    links::run(&context).await;
    rls::run(&context, &database_url).await;
}

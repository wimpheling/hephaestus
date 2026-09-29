use serial_test::serial;
use std::env;
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 88;

use super::support::{
    admin_pool, assert_active_owner_key_is_unique, assert_application_rls, assert_composite_fks,
    assert_immutability_and_removed_terminal, assert_installation_identity_is_immutable,
    assert_role, assert_worker_grants, role_pool, seed_installation_rows, seed_parent_rows,
};

#[tokio::test]
#[serial]
// Keep the schema lifecycle proof together so foreign keys, immutability, and
// row-level security are validated against the same fixture and transaction state.
#[allow(clippy::too_many_lines)]
async fn ui_installation_schema_enforces_fks_lifecycle_immutability_and_rls() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI installation schema: test URL is unset");
        return;
    };
    let bootstrap = admin_pool(&database_url).await;
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0088");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker exists");
    assert!(max_migration >= EXPECTED_MIGRATION);

    let worker = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let app = role_pool(&database_url, "hephaestus_app", Uuid::nil()).await;
    assert_role(&worker, "hephaestus_worker", false, true).await;
    assert_role(&app, "hephaestus_app", false, false).await;

    let fixture = seed_parent_rows(&worker).await;
    seed_installation_rows(&worker, &fixture).await;
    assert_worker_grants(&worker).await;
    assert_installation_identity_is_immutable(&worker, &fixture).await;
    assert_active_owner_key_is_unique(&worker, &fixture).await;
    assert_composite_fks(&worker, &fixture).await;
    assert_immutability_and_removed_terminal(&worker, &fixture).await;
    assert_application_rls(&database_url, &fixture).await;

    println!(
        "REAL_RELEASE_UI_INSTALLATION_SCHEMA=1 migration={max_migration} \
         app_role=hephaestus_app worker_role=hephaestus_worker \
         partial_unique=1 composite_fks=1 immutable=1 removed_terminal=1 app_rls=1"
    );
}

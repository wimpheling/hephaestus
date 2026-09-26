//! Opt-in real-PostgreSQL tests for Mélange evaluation and RLS.

use authz_postgres::PostgresMelangeAuthorizer;
use serial_test::serial;

#[path = "postgres/catalog.rs"]
mod catalog;
#[path = "postgres/parity.rs"]
mod parity;
#[path = "postgres/permission_checks.rs"]
mod permission_checks;
#[path = "postgres/revocation.rs"]
mod revocation;
#[path = "postgres/rls_checks.rs"]
mod rls_checks;
#[path = "postgres/seed.rs"]
mod seed;
#[path = "postgres/support.rs"]
mod support;

#[tokio::test]
#[serial]
async fn generated_permissions_and_rls_enforce_the_same_perimeter() {
    let Some(pool) = support::pool().await else {
        return;
    };
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply Phase 3 migrations");
    let fixture = seed::seed(&pool).await;
    let authorizer = PostgresMelangeAuthorizer;
    permission_checks::run(&pool, &fixture, &authorizer).await;
    rls_checks::run(&pool, &fixture, &authorizer).await;
    revocation::run(&pool, &fixture, &authorizer).await;
    catalog::run(&pool, &fixture).await;
}

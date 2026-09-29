//! Real-role migration and verifier coverage for human browser sessions.
//!
//! This opt-in test needs a disposable `PostgreSQL` instance with the application
//! and worker roles. The validation runner must provide the test URL and assert
//! the `REAL_BROWSER_SESSION_SCHEMA=1` marker.

use serial_test::serial;

#[path = "browser_session_schema/support.rs"]
mod support;

#[tokio::test]
#[serial]
async fn browser_session_schema_enforces_roles_lifecycle_and_verifier() {
    let Some((fixture, max_migration)) = support::prepare().await else {
        return;
    };
    let revoked_id = support::exercise_lifecycle(&fixture).await;
    support::exercise_roles_and_constraints(&fixture, revoked_id).await;
    println!(
        "REAL_BROWSER_SESSION_SCHEMA=1 migration={max_migration} app_role=hephaestus_app verifier=active_expiry_revocation"
    );
}

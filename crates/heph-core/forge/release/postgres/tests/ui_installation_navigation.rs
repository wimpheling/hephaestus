//! Real-PostgreSQL navigation projection matrix candidate.

#[path = "ui_installation_navigation/support.rs"]
mod support;

#[tokio::test]
#[serial_test::serial]
async fn list_ui_installations_enforces_explicit_org_target_and_visibility() {
    support::run().await;
}

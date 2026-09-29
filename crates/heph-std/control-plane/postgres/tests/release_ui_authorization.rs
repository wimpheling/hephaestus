//! Real application-role inspection of immutable release UI bindings.

#[path = "release_ui_authorization/support.rs"]
mod support;

#[tokio::test]
async fn get_release_inspects_ui_rows_through_application_role() {
    support::run().await;
}

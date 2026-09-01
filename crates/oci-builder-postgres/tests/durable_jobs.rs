//! Structural regression tests for the worker-owned migration boundary.

const MIGRATION: &str = include_str!("../../../migrations/0017_repository_oci_image_jobs.sql");
const PREPARATION_HISTORY_FIX: &str =
    include_str!("../../../migrations/0051_fix_repository_oci_image_preparation_event_outcome.sql");
const PREPARATION_EVENT_GRANT: &str = include_str!(
    "../../../migrations/0052_grant_repository_oci_preparation_event_trigger_insert.sql"
);
const DEFINITION_TRIGGER_READ_GRANT: &str =
    include_str!("../../../migrations/0053_grant_repository_oci_definition_trigger_read.sql");

#[test]
fn production_is_transactionally_enqueued_and_worker_owned() {
    assert!(MIGRATION.contains("enqueue_repository_oci_image_production_job"));
    assert!(
        MIGRATION.contains("AFTER INSERT OR UPDATE OF status ON repository_oci_image_definitions")
    );
    assert!(MIGRATION.contains("TG_OP = 'INSERT'"));
    assert!(MIGRATION.contains("repository_oci_image_production_jobs_worker"));
    assert!(MIGRATION.contains("current_user = 'hephaestus_worker'"));
}

#[test]
fn only_successful_worker_output_can_make_an_image_ready() {
    assert!(MIGRATION.contains("verify_repository_oci_image_worker_completion"));
    assert!(MIGRATION.contains("job.state = 'succeeded'"));
    assert!(MIGRATION.contains("oci_image_materialization_jobs"));
    assert!(MIGRATION.contains("state = 'materialized'"));
}

#[test]
fn preparation_history_uses_a_searched_case_for_pending_states() {
    assert!(
        PREPARATION_HISTORY_FIX.contains("WHEN NEW.state IN ('queued', 'claimed') THEN 'pending'")
    );
    assert!(!PREPARATION_HISTORY_FIX.contains("CASE NEW.state\n            WHEN NEW.state IN"));
}

#[test]
fn preparation_history_trigger_owner_can_insert_events() {
    assert!(PREPARATION_EVENT_GRANT.contains(
        "GRANT INSERT ON repository_oci_image_preparation_events TO hephaestus_authz_owner"
    ));
}

#[test]
fn ready_event_trigger_owner_can_read_the_image_definition() {
    assert!(
        DEFINITION_TRIGGER_READ_GRANT
            .contains("GRANT SELECT ON repository_oci_image_definitions TO hephaestus_authz_owner")
    );
}

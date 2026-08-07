//! Structural regression tests for the worker-owned migration boundary.

const MIGRATION: &str = include_str!("../../../migrations/0017_repository_oci_image_jobs.sql");

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

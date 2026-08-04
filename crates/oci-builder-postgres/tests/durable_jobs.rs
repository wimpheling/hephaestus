//! Structural regression tests for the worker-owned migration boundary.

const MIGRATION: &str =
    include_str!("../../../migrations/0017_project_builder_preparation_jobs.sql");

#[test]
fn preparation_is_transactionally_enqueued_and_worker_owned() {
    assert!(MIGRATION.contains("enqueue_project_builder_preparation_job"));
    assert!(MIGRATION.contains("AFTER INSERT OR UPDATE OF status ON project_builder_definitions"));
    assert!(MIGRATION.contains("TG_OP = 'INSERT'"));
    assert!(MIGRATION.contains("project_builder_preparation_jobs_worker"));
    assert!(MIGRATION.contains("current_user = 'hephaestus_worker'"));
}

#[test]
fn only_successful_worker_output_can_make_a_builder_ready() {
    assert!(MIGRATION.contains("verify_project_builder_worker_completion"));
    assert!(MIGRATION.contains("job.state = 'succeeded'"));
    assert!(MIGRATION.contains("project_builder_root_materialization_jobs"));
    assert!(MIGRATION.contains("state = 'materialized'"));
}

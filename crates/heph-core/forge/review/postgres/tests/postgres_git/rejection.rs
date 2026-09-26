use forge_service::GitStorage;
use review_domain::ControlKind;
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::{ControlOutcome, ReviewControlService};
use runtime_types::RunId;
use serial_test::serial;
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{insert_control, pool, seed};

#[tokio::test]
#[serial]
async fn missing_retry_source_is_terminally_rejected_and_replay_is_idempotent() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");
    let temporary = TempDir::new().expect("temporary fixture");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("Git storage"),
    );
    let fixture = seed(&pool, &storage, &temporary).await;
    let orphan_run_id = RunId::new();
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, command_id, state, requires_state,
          created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7, 'cleaned_up', false,
                 now(), now())",
    )
    .bind(orphan_run_id.as_uuid())
    .bind(fixture.instance_id)
    .bind(fixture.instance_revision_id)
    .bind(fixture.release_id)
    .bind(fixture.release_agent_id)
    .bind(fixture.attachment_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("orphan run");
    let service = ReviewControlService::new(
        Arc::new(PostgresReviewRepository::new(pool.clone())),
        Arc::new(GitRepositoryLocator::new(Arc::clone(&storage))),
    );
    let retry = insert_control(
        &pool,
        &fixture,
        ControlKind::RetryRun,
        Some(orphan_run_id),
        None,
    )
    .await;
    assert_eq!(
        service
            .execute(&retry)
            .await
            .expect("reject unsupported retry"),
        ControlOutcome::Rejected
    );
    let (state, diagnostics): (String, serde_json::Value) =
        sqlx::query_as("SELECT state, diagnostics FROM control_requests WHERE id = $1")
            .bind(retry.command_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("terminal control request");
    assert_eq!(state, "failed");
    assert_eq!(
        diagnostics,
        serde_json::json!([{ "code": "retry_unsupported" }])
    );
    let retry_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE retry_of_run_id = $1")
            .bind(orphan_run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("retry request count");
    assert_eq!(retry_count, 0);
    assert_eq!(
        service
            .execute(&retry)
            .await
            .expect("replay rejected retry"),
        ControlOutcome::AlreadyCompleted
    );
    let rejected_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE scope_kind = 'run' AND scope_id = $1
           AND aggregate_type = 'run' AND event_type = 'run.changed'
           AND safe_state = 'failed'",
    )
    .bind(orphan_run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("durable rejection event count");
    assert!(rejected_events >= 1);
}

use forge_service::GitStorage;
use review_domain::ControlKind;
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::{ControlOutcome, ReviewControlService};
use serial_test::serial;
use std::sync::Arc;
use tempfile::TempDir;

use super::support::{git_text, insert_control, pool, seed};

#[tokio::test]
#[serial]
async fn authorized_controls_publish_a_cas_result_and_durable_run_commands() {
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
    let repository_adapter = Arc::new(PostgresReviewRepository::new(pool.clone()));
    let locator = Arc::new(GitRepositoryLocator::new(Arc::clone(&storage)));
    let service = ReviewControlService::new(repository_adapter, locator);

    let approval = insert_control(
        &pool,
        &fixture,
        ControlKind::ApproveResult,
        None,
        Some(fixture.proposal_id),
    )
    .await;
    assert_eq!(
        service.execute(&approval).await.expect("approve result"),
        ControlOutcome::Completed
    );
    assert_eq!(
        git_text(
            &storage.repository_path(fixture.repository_id),
            &["rev-parse", "refs/heads/main"]
        ),
        fixture.result_commit
    );
    assert_eq!(
        service
            .execute(&approval)
            .await
            .expect("duplicate approval"),
        ControlOutcome::AlreadyCompleted
    );
    let proposal_state: String =
        sqlx::query_scalar("SELECT state FROM review_proposals WHERE id = $1")
            .bind(fixture.proposal_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("proposal state");
    assert_eq!(proposal_state, "approved");

    let retry = insert_control(
        &pool,
        &fixture,
        ControlKind::RetryRun,
        Some(fixture.run_id),
        None,
    )
    .await;
    assert_eq!(
        service.execute(&retry).await.expect("retry run"),
        ControlOutcome::Completed
    );
    assert_eq!(
        service.execute(&retry).await.expect("duplicate retry run"),
        ControlOutcome::AlreadyCompleted
    );
    let retry_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE retry_of_run_id = $1")
            .bind(fixture.run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("retry request");
    assert_eq!(retry_count, 1);
    let start_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_type = 'run_request'
           AND subject = 'hephaestus.run.start'
           AND payload ->> 'run_id' IN (
               SELECT run_id::text FROM run_requests WHERE retry_of_run_id = $1
           )",
    )
    .bind(fixture.run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("retry outbox");
    assert_eq!(start_events, 1);

    let cancellation = insert_control(
        &pool,
        &fixture,
        ControlKind::CancelRun,
        Some(fixture.run_id),
        None,
    )
    .await;
    assert_eq!(
        service
            .execute(&cancellation)
            .await
            .expect("cancel command"),
        ControlOutcome::Completed
    );
    assert_eq!(
        service
            .execute(&cancellation)
            .await
            .expect("duplicate cancel command"),
        ControlOutcome::AlreadyCompleted
    );
    let cancel_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_id = $1 AND subject = 'heph.run.command.cancel.v1'",
    )
    .bind(fixture.run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("cancel outbox");
    assert_eq!(cancel_events, 1);
}

use forge_service::GitStorage;
use review_domain::ControlKind;
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::{ControlOutcome, ReviewControlService};
use serial_test::serial;
use std::sync::Arc;
use tempfile::TempDir;

use super::support::{git_text, git_text_with_identity, insert_control, pool, run_git, seed};

#[tokio::test]
#[serial]
async fn approval_marks_a_proposal_conflicted_when_the_target_moved() {
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
    let repository = storage.repository_path(fixture.repository_id);
    let input_tree = format!("{}^{}tree{}", fixture.input_commit, '{', '}');
    let tree = git_text(&repository, &["rev-parse", &input_tree]);
    let concurrent = git_text_with_identity(
        &repository,
        &[
            "commit-tree",
            &tree,
            "-p",
            &fixture.input_commit,
            "-m",
            "concurrent update",
        ],
    );
    run_git(
        &repository,
        &[
            "update-ref",
            "refs/heads/main",
            &concurrent,
            &fixture.input_commit,
        ],
    );
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
        service
            .execute(&approval)
            .await
            .expect("conflicted approval"),
        ControlOutcome::Conflicted
    );
    assert_eq!(
        git_text(&repository, &["rev-parse", "refs/heads/main"]),
        concurrent
    );
    let proposal_state: String =
        sqlx::query_scalar("SELECT state FROM review_proposals WHERE id = $1")
            .bind(fixture.proposal_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("proposal state");
    assert_eq!(proposal_state, "conflicted");
}

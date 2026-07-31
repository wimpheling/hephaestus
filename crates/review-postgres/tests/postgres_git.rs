//! Real `PostgreSQL` and bare-Git verification for durable review controls.

use forge_domain::RepositoryId;
use forge_service::GitStorage;
use identity_domain::{RequestId, UserId};
use review_domain::{ControlCommand, ControlKind, ControlRequestId, ReviewProposalId};
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::{ControlOutcome, ReviewControlService};
use runtime_types::RunId;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{path::Path, process::Command, sync::Arc};
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn authorized_controls_publish_a_cas_result_and_durable_run_commands() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../migrations")
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

#[tokio::test]
#[serial]
async fn approval_marks_a_proposal_conflicted_when_the_target_moved() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../migrations")
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

struct Fixture {
    actor_id: UserId,
    repository_id: RepositoryId,
    run_id: RunId,
    proposal_id: ReviewProposalId,
    input_commit: String,
    result_commit: String,
}

// Keeping the relational and Git fixture together makes its provenance
// invariant visible to the approval tests.
#[allow(clippy::too_many_lines)]
async fn seed(pool: &PgPool, storage: &GitStorage, temporary: &TempDir) -> Fixture {
    let actor_id = UserId::new();
    let organization_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let repository_id = RepositoryId::new();
    let family_id = Uuid::new_v4();
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let instance_revision_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    let run_id = RunId::new();
    let receive_id = Uuid::new_v4();
    let result_id = Uuid::new_v4();
    let proposal_id = ReviewProposalId::new();

    storage
        .create_bare(repository_id, "main")
        .await
        .expect("create bare repository");
    let work = temporary.path().join("work");
    run_git(
        temporary.path(),
        &["init", work.to_str().expect("UTF-8 path")],
    );
    run_git(&work, &["config", "user.name", "Fixture"]);
    run_git(&work, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(work.join("README.md"), "input\n").expect("write input");
    run_git(&work, &["add", "README.md"]);
    run_git(&work, &["commit", "-m", "input"]);
    let input_commit = git_text(&work, &["rev-parse", "HEAD"]);
    run_git(
        &work,
        &[
            "remote",
            "add",
            "origin",
            storage
                .repository_path(repository_id)
                .to_str()
                .expect("UTF-8 repository"),
        ],
    );
    run_git(&work, &["push", "origin", "HEAD:refs/heads/main"]);
    std::fs::write(work.join("README.md"), "input\nagent result\n").expect("write result");
    run_git(&work, &["commit", "-am", "result"]);
    let result_commit = git_text(&work, &["rev-parse", "HEAD"]);
    let result_ref = format!("refs/heads/hephaestus/{instance_id}/{run_id}");
    run_git(&work, &["push", "origin", &format!("HEAD:{result_ref}")]);

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Reviewer')")
        .bind(actor_id.as_uuid())
        .execute(pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Fixture')")
        .bind(organization_id)
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("owner membership");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $2, 'project')",
    )
    .bind(project_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("project");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2)",
    )
    .bind(project_id)
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("project maintainer");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, 'repository')",
    )
    .bind(repository_id.as_uuid())
    .bind(project_id)
    .execute(pool)
    .await
    .expect("repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'reviewer')",
    )
    .bind(family_id)
    .bind(repository_id.as_uuid())
    .execute(pool)
    .await
    .expect("agent family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_id)
    .bind(repository_id.as_uuid())
    .bind(&input_commit)
    .bind([1_u8; 32].as_slice())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, publication_actor_id,
          published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5, '{}',
                 $6, $7, 'published', $8, now())",
    )
    .bind(release_id)
    .bind(repository_id.as_uuid())
    .bind(&input_commit)
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'reviewer', 'Reviewer', $4, $5, '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(serde_json::json!({
        "command": "bin/reviewer",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "fixture"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state, created_by)
         VALUES ($1, $2, $3, 'reviewer', 'active', $4)",
    )
    .bind(instance_id)
    .bind(project_id)
    .bind(family_id)
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable, created_by)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8,
                 'fixture/v1', true, $9)",
    )
    .bind(instance_revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind(serde_json::json!({"network": "disabled"}))
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 128, "network": "disabled"}))
    .bind([6_u8; 32].as_slice())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("instance revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(instance_revision_id)
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy, created_by)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', $5)",
    )
    .bind(attachment_id)
    .bind(instance_id)
    .bind(project_id)
    .bind(repository_id.as_uuid())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("attachment");
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, command_id, state, outcome, requires_state,
          created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7,
                 'cleaned_up', 'succeeded', false, now(), now())",
    )
    .bind(run_id.as_uuid())
    .bind(instance_id)
    .bind(instance_revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(attachment_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("run");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'fixture', 'accepted', now())",
    )
    .bind(receive_id)
    .bind(repository_id.as_uuid())
    .bind(actor_id.as_uuid())
    .execute(pool)
    .await
    .expect("receive");
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, receive_id,
          run_id, command_id, actor_id, request_id, instance_id,
          instance_revision_id, release_id, release_agent_id, attachment_id,
          platform_policy_version, requires_state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $6, $7, $8,
                 $9, $10, $11, $12, $13, 'fixture/v1', false)",
    )
    .bind(Uuid::new_v4())
    .bind(repository_id.as_uuid())
    .bind(&input_commit)
    .bind(receive_id)
    .bind(run_id.as_uuid())
    .bind(Uuid::new_v4())
    .bind(actor_id.as_uuid())
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(instance_revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(attachment_id)
    .execute(pool)
    .await
    .expect("run request");
    sqlx::query(
        "INSERT INTO run_results
         (id, run_id, repository_id, instance_id, input_commit, result_commit,
          result_ref, message, state, completed_at, instance_revision_id,
          release_id, release_agent_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'fixture result',
                 'completed', now(), $8, $9, $10)",
    )
    .bind(result_id)
    .bind(run_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(instance_id)
    .bind(&input_commit)
    .bind(&result_commit)
    .bind(&result_ref)
    .bind(instance_revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .execute(pool)
    .await
    .expect("result");
    let generated_proposal: Uuid =
        sqlx::query_scalar("SELECT id FROM review_proposals WHERE result_id = $1")
            .bind(result_id)
            .fetch_one(pool)
            .await
            .expect("generated proposal");
    sqlx::query("UPDATE review_proposals SET id = $2 WHERE id = $1")
        .bind(generated_proposal)
        .bind(proposal_id.as_uuid())
        .execute(pool)
        .await
        .expect("stable proposal ID");

    Fixture {
        actor_id,
        repository_id,
        run_id,
        proposal_id,
        input_commit,
        result_commit,
    }
}

async fn insert_control(
    pool: &PgPool,
    fixture: &Fixture,
    kind: ControlKind,
    run_id: Option<RunId>,
    proposal_id: Option<ReviewProposalId>,
) -> ControlCommand {
    let command = ControlCommand {
        command_id: ControlRequestId::new(),
        kind,
        actor_id: fixture.actor_id,
        request_id: RequestId::new(),
        repository_id: fixture.repository_id,
        run_id,
        proposal_id,
        reason: String::from("fixture"),
    };
    sqlx::query(
        "INSERT INTO control_requests
         (id, kind, actor_id, request_id, repository_id, run_id, proposal_id, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(command.command_id.as_uuid())
    .bind(kind_name(kind))
    .bind(command.actor_id.as_uuid())
    .bind(command.request_id.as_uuid())
    .bind(command.repository_id.as_uuid())
    .bind(command.run_id.map(RunId::as_uuid))
    .bind(command.proposal_id.map(ReviewProposalId::as_uuid))
    .bind(&command.reason)
    .execute(pool)
    .await
    .expect("control request");
    command
}

fn run_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn git_text_with_identity(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .env("GIT_AUTHOR_NAME", "Concurrent Reviewer")
        .env("GIT_AUTHOR_EMAIL", "concurrent@example.invalid")
        .env("GIT_COMMITTER_NAME", "Concurrent Reviewer")
        .env("GIT_COMMITTER_EMAIL", "concurrent@example.invalid")
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

const fn kind_name(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::CancelRun => "cancel_run",
        ControlKind::RetryRun => "retry_run",
        ControlKind::ApproveResult => "approve_result",
        ControlKind::RejectResult => "reject_result",
    }
}

async fn pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("PostgreSQL connection"),
    )
}

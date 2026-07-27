//! Opt-in native smart-HTTP and `PostgreSQL` integration coverage.

use async_trait::async_trait;
use forge_domain::GitRef;
use forge_service::{CreateRepository, GitStorage, PgForgeRepository, RUN_START_SUBJECT};
use git_http::{
    AuthorizationError, AuthorizationRequest, GitAuthorizer, GitHttpLimits, GitHttpService,
    GitOperation, Principal,
};
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{path::Path, sync::Arc};
use tokio::{process::Command, sync::Mutex};

struct RecordingAuthorizer {
    calls: Mutex<Vec<GitOperation>>,
}

#[async_trait]
impl GitAuthorizer for RecordingAuthorizer {
    async fn authorize(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<Principal, AuthorizationError> {
        self.calls.lock().await.push(request.operation);
        Ok(Principal {
            name: String::from("integration-user"),
        })
    }
}

#[tokio::test]
#[serial]
async fn clone_fetch_push_audit_and_run_request() {
    let Some(pool) = postgres_pool().await else {
        eprintln!("skipping: HEPHAESTUS_POSTGRES_TEST_URL is not set");
        return;
    };
    let temporary = tempfile::tempdir().expect("temporary directory");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("Git storage"),
    );
    let repository_service = Arc::new(PgForgeRepository::new(pool.clone(), Arc::clone(&storage)));
    repository_service
        .initialize()
        .await
        .expect("forge migrations");
    let project = repository_service
        .create_project("smart-http-integration")
        .await
        .expect("project");
    let repository = repository_service
        .create_repository(&CreateRepository {
            project_id: project.id,
            name: String::from("transport"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            agent_runs_enabled: true,
        })
        .await
        .expect("repository");

    let authorizer = Arc::new(RecordingAuthorizer {
        calls: Mutex::new(Vec::new()),
    });
    let backend = git_exec_path().await.join("git-http-backend");
    let router = GitHttpService::new(
        Arc::clone(&repository_service),
        Arc::clone(&storage),
        authorizer.clone(),
        backend,
        GitHttpLimits::default(),
    )
    .router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("smart HTTP server");
    });
    let remote = format!("http://{address}/{}", repository.id);

    let source = temporary.path().join("source");
    tokio::fs::create_dir(&source)
        .await
        .expect("source directory");
    git(&source, &["init", "--initial-branch=main"]).await;
    git(&source, &["config", "user.name", "Hephaestus Test"]).await;
    git(
        &source,
        &["config", "user.email", "hephaestus@example.invalid"],
    )
    .await;
    tokio::fs::write(source.join("agent.toml"), valid_agent_config())
        .await
        .expect("agent.toml");
    tokio::fs::write(source.join("README.md"), "# transport\n")
        .await
        .expect("README");
    git(&source, &["add", "."]).await;
    git(&source, &["commit", "-m", "add agent"]).await;
    let commit = git_output(&source, &["rev-parse", "HEAD"]).await;
    git(&source, &["remote", "add", "origin", &remote]).await;
    git(&source, &["push", "origin", "HEAD:refs/heads/main"]).await;

    let clone = temporary.path().join("clone");
    git(
        temporary.path(),
        &["clone", &remote, clone.to_str().expect("UTF-8 clone path")],
    )
    .await;
    git(&clone, &["fetch", "origin"]).await;

    let receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(repository.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("receive count");
    let update_commit: String = sqlx::query_scalar(
        "SELECT u.new_commit
         FROM git_ref_updates u
         JOIN git_receives r ON r.id = u.receive_id
         WHERE r.repository_id = $1 AND u.git_ref = 'refs/heads/main'",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("audited commit");
    let revision_commit: String = sqlx::query_scalar(
        "SELECT commit_sha FROM agent_config_revisions WHERE repository_id = $1",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("configuration revision");
    let run_request_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE repository_id = $1")
            .bind(repository.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("run-request count");
    let start_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_type = 'forge' AND subject = $1
           AND aggregate_id IN (SELECT id FROM run_requests WHERE repository_id = $2)",
    )
    .bind(RUN_START_SUBJECT)
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("start-event count");

    assert_eq!(receive_count, 1);
    assert_eq!(update_commit, commit);
    assert_eq!(revision_commit, commit);
    assert_eq!(run_request_count, 1);
    assert_eq!(start_event_count, 1);
    let calls = authorizer.calls.lock().await.clone();
    assert!(calls.contains(&GitOperation::Clone));
    assert!(calls.contains(&GitOperation::Fetch));
    assert!(calls.contains(&GitOperation::Push));

    server.abort();
}

async fn postgres_pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("PostgreSQL test connection"),
    )
}

async fn git_exec_path() -> std::path::PathBuf {
    std::path::PathBuf::from(git_output(Path::new("."), &["--exec-path"]).await)
}

async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 Git output")
        .trim()
        .to_owned()
}

const fn valid_agent_config() -> &'static str {
    r#"
version = 1

[agent]
name = "reviewer"

[guest]
command = "/usr/bin/review"
arguments = ["--format=json"]
working_directory = "/workspace"

[resources]
vcpus = 2
memory_mib = 512

[root_image]
reference = "registry.example/agent@sha256:abc"

[workspace]
mount = true
path = "/workspace"
read_only = true

[state_volume]
enabled = true

[network]
profile = "disabled"

[triggers]
push = true
refs = ["refs/heads/*"]
"#
}

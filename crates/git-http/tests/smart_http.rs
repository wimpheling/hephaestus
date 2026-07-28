//! Opt-in native smart-HTTP and `PostgreSQL` integration coverage.

use async_trait::async_trait;
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{GitRef, OrganizationId};
use forge_service::{CreateRepository, GitStorage, PgForgeRepository, RUN_START_SUBJECT};
use git_http::{
    AuthenticationError, AuthorizationError, AuthorizationRequest, GitAuthenticator, GitAuthorizer,
    GitHttpLimits, GitHttpService, GitOperation, PostgresGitAuthorizer, Principal,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{path::Path, sync::Arc};
use tokio::{process::Command, sync::Mutex};

struct RecordingAuthorizer {
    calls: Mutex<Vec<GitOperation>>,
    identity: AuthenticatedIdentity,
}

struct TestIdentityProvider {
    identity: AuthenticatedIdentity,
}

#[async_trait]
impl GitAuthenticator for TestIdentityProvider {
    async fn authenticate(
        &self,
        _credential: Option<&str>,
        request_id: RequestId,
    ) -> Result<Principal, AuthenticationError> {
        let mut identity = self.identity.clone();
        identity.request_id = request_id;
        Ok(Principal {
            name: identity.subject.clone(),
            identity,
        })
    }
}

#[tokio::test]
#[serial]
async fn postgres_authorizer_allows_reads_and_rejects_push_without_write() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Phase 3 migrations");
    let owner = UserId::new();
    let member = UserId::new();
    let organization = OrganizationId::new();
    let project = forge_domain::ProjectId::new();
    let repository = forge_domain::RepositoryId::new();
    for (user, name) in [(owner, "owner"), (member, "member")] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind(name)
            .execute(&pool)
            .await
            .expect("user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'git-authz')")
        .bind(organization.as_uuid())
        .execute(&pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner'), ($1, $3, 'member')",
    )
    .bind(organization.as_uuid())
    .bind(owner.as_uuid())
    .bind(member.as_uuid())
    .execute(&pool)
    .await
    .expect("memberships");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $2, 'git-authz')",
    )
    .bind(project.as_uuid())
    .bind(organization.as_uuid())
    .execute(&pool)
    .await
    .expect("project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.as_uuid())
        .bind(owner.as_uuid())
        .execute(&pool)
        .await
        .expect("maintainer");
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch)
         VALUES ($1, $2, 'git-authz', 'refs/heads/main')",
    )
    .bind(repository.as_uuid())
    .bind(project.as_uuid())
    .execute(&pool)
    .await
    .expect("repository");

    let member_identity = AuthenticatedIdentity::new(
        member,
        "https://issuer.example",
        "member",
        json!({}),
        RequestId::new(),
    );
    let member_authorizer = PostgresGitAuthorizer::new(pool.clone());
    for operation in [GitOperation::Clone, GitOperation::Fetch] {
        member_authorizer
            .authorize(&AuthorizationRequest {
                repository_id: repository,
                operation,
                identity: member_identity.clone(),
            })
            .await
            .expect("organization member may read Git");
    }
    assert!(
        member_authorizer
            .authorize(&AuthorizationRequest {
                repository_id: repository,
                operation: GitOperation::Push,
                identity: member_identity,
            })
            .await
            .is_err()
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE actor_id = $1 AND object_id = $2",
    )
    .bind(member.as_uuid())
    .bind(repository.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("authorization audits");
    assert_eq!(audits, 3);
}

#[async_trait]
impl GitAuthorizer for RecordingAuthorizer {
    async fn authorize(&self, request: &AuthorizationRequest) -> Result<(), AuthorizationError> {
        self.calls.lock().await.push(request.operation);
        Ok(())
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
    let repository_service = Arc::new(
        PgForgeRepository::new(pool.clone(), Arc::clone(&storage))
            .with_authorizer(Arc::new(PostgresMelangeAuthorizer)),
    );
    repository_service
        .initialize()
        .await
        .expect("forge migrations");
    let organization_id = OrganizationId::new();
    let user_id = UserId::new();
    let identity = AuthenticatedIdentity::new(
        user_id,
        "https://issuer.example",
        "integration-user",
        json!({}),
        RequestId::new(),
    );
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'integration-user')")
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind("smart-http-integration")
        .execute(&pool)
        .await
        .expect("organization");
    let project = repository_service
        .create_project_trusted(organization_id, "smart-http-integration")
        .await
        .expect("project");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await
    .expect("organization owner");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.id.as_uuid())
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("project maintainer");
    repository_service
        .create_project(&identity, organization_id, "authorized-project")
        .await
        .expect("authorized project creation");
    let repository = repository_service
        .create_repository(
            &identity,
            &CreateRepository {
                project_id: project.id,
                name: String::from("transport"),
                default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
                is_public: false,
                agent_runs_enabled: true,
            },
        )
        .await
        .expect("repository");

    let identity = identity.clone();
    let authorizer = Arc::new(RecordingAuthorizer {
        calls: Mutex::new(Vec::new()),
        identity: identity.clone(),
    });
    let authenticator = Arc::new(TestIdentityProvider { identity });
    let backend = git_exec_path().await.join("git-http-backend");
    let router = GitHttpService::new(
        Arc::clone(&repository_service),
        Arc::clone(&storage),
        authenticator,
        authorizer.clone(),
        backend,
        GitHttpLimits::default(),
    )
    .expect("Git HTTP configuration")
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
    let run_authorization_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE actor_id = $1 AND permission = 'can_execute'",
    )
    .bind(user_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("run authorization count");

    assert_eq!(receive_count, 1);
    assert_eq!(update_commit, commit);
    assert_eq!(revision_commit, commit);
    assert_eq!(run_request_count, 1);
    assert_eq!(start_event_count, 1);
    assert_eq!(run_authorization_count, 1);
    let calls = authorizer.calls.lock().await.clone();
    assert!(calls.contains(&GitOperation::Clone));
    assert!(calls.contains(&GitOperation::Fetch));
    assert!(calls.contains(&GitOperation::Push));

    let deletable = repository_service
        .create_repository(
            &authorizer.identity,
            &CreateRepository {
                project_id: project.id,
                name: String::from("deletable"),
                default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
                is_public: false,
                agent_runs_enabled: false,
            },
        )
        .await
        .expect("authorized deletable repository");
    let deletable_path = storage.repository_path(deletable.id);
    repository_service
        .delete_repository(&authorizer.identity, deletable.id)
        .await
        .expect("authorized repository deletion");
    assert!(!deletable_path.exists());

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
path = "/workspace/repo"
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

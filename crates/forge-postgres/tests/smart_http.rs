//! Opt-in native smart-HTTP and `PostgreSQL` integration coverage.

use async_trait::async_trait;
use authz_postgres::PostgresMelangeAuthorizer;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use forge_domain::{GitRef, OrganizationId};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage, RUN_START_SUBJECT};
use git_capability_domain::{
    BoundGitCapability, BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitOperation as CapabilityGitOperation, RefGlob,
    RefMutationPermission, RefNamespacePolicy, RefUpdatePolicy,
    RepositoryId as CapabilityRepositoryId, TransferLimits,
};
use git_http::{
    AuthenticationError, AuthorizationError, AuthorizationRequest, CompositeGitAuthenticator,
    GitAuthenticator, GitAuthorizer, GitHttpLimits, GitHttpService, GitOperation,
    PostgresGitAuthorizer, Principal, RuntimeGitHttpAuthenticator,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use pat_domain::{PersonalAccessTokenLabel, PersonalAccessTokenScope};
use pat_postgres::{CreatePersonalAccessToken, PostgresPersonalAccessTokenService};
use runtime_git_authority::{RuntimeGitCredential, RuntimeGitCredentialIssuer};
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use runtime_handoff_local::EncryptedFileRuntimeGitHandoffStore;
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{process::Command, sync::Mutex};
use uuid::Uuid;

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
        Ok(Principal::human(identity))
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
    let member_authorizer = PostgresGitAuthorizer::new(Arc::new(
        authz_postgres::PostgresGitAuthorizer::new(pool.clone()),
    ));
    for operation in [GitOperation::Clone, GitOperation::Fetch] {
        member_authorizer
            .authorize(&AuthorizationRequest {
                repository_id: repository,
                operation,
                principal: Principal::human(member_identity.clone()),
            })
            .await
            .expect("organization member may read Git");
    }
    assert!(
        member_authorizer
            .authorize(&AuthorizationRequest {
                repository_id: repository,
                operation: GitOperation::Push,
                principal: Principal::human(member_identity),
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
    // Apply the current workspace schema before constructing the repository
    // service so this focused matrix covers the complete receive-path schema.
    let migrations = sqlx::migrate!("../../migrations");
    migrations
        .run(&pool)
        .await
        .expect("current workspace migrations");
    let latest_migration: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .expect("latest applied migration");
    assert_eq!(
        latest_migration, 98,
        "focused matrix must use current schema"
    );
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
    let other_repository = repository_service
        .create_repository(
            &identity,
            &CreateRepository {
                project_id: project.id,
                name: String::from("other-transport"),
                default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
                is_public: false,
                agent_runs_enabled: false,
            },
        )
        .await
        .expect("other repository");

    let identity = identity.clone();
    let authorizer = Arc::new(RecordingAuthorizer {
        calls: Mutex::new(Vec::new()),
        identity: identity.clone(),
    });
    let issued_pat = PostgresPersonalAccessTokenService::new(pool.clone())
        .create(
            &identity,
            CreatePersonalAccessToken {
                label: PersonalAccessTokenLabel::parse("smart HTTP integration")
                    .expect("valid PAT label"),
                scope: PersonalAccessTokenScope::new(
                    [
                        git_capability_domain::GitOperation::Discover,
                        git_capability_domain::GitOperation::Fetch,
                        git_capability_domain::GitOperation::Receive,
                    ],
                    Some([repository.id]),
                )
                .expect("exact PAT scope"),
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
            },
        )
        .await
        .expect("issue smart HTTP PAT");
    let basic_credential = format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("heph-pat:{}", issued_pat.token.expose()))
    );
    let oidc_authenticator: Arc<dyn GitAuthenticator> = Arc::new(TestIdentityProvider { identity });
    let authenticator = Arc::new(CompositeGitAuthenticator::new(
        oidc_authenticator,
        Arc::new(PostgresPersonalAccessTokenService::new(pool.clone())),
    ));
    let backend = git_exec_path().await.join("git-http-backend");
    let router = GitHttpService::new(
        Arc::clone(&repository_service),
        Arc::clone(&storage),
        authenticator,
        authorizer.clone(),
        backend.clone(),
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
    seed_attached_instance(&pool, user_id, project.id, repository.id, &commit).await;
    let instance_id: Uuid =
        sqlx::query_scalar("SELECT instance_id FROM agent_attachments WHERE repository_id = $1")
            .bind(repository.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("transport origin attachment");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy, enabled, created_by)
         VALUES ($1, $2, $3, $4, 'refs/heads/*', 'push', true, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(project.id.as_uuid())
    .bind(repository.id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await
    .expect("transport sibling attachment");
    git(&source, &["remote", "add", "origin", &remote]).await;
    git_authenticated(
        &source,
        &["push", "origin", "HEAD:refs/heads/main"],
        &basic_credential,
    )
    .await;

    let clone = temporary.path().join("clone");
    git_authenticated(
        temporary.path(),
        &["clone", &remote, clone.to_str().expect("UTF-8 clone path")],
        &basic_credential,
    )
    .await;
    git_authenticated(&clone, &["fetch", "origin"], &basic_credential).await;

    tokio::fs::write(source.join("runtime.txt"), "runtime receive\n")
        .await
        .expect("runtime change");
    git(&source, &["add", "runtime.txt"]).await;
    git(&source, &["commit", "-m", "runtime receive"]).await;
    let runtime_commit = git_output(&source, &["rev-parse", "HEAD"]).await;
    let (runtime_session_id, origin_attachment, runtime_credential, runtime_binding_id) =
        seed_runtime_transport_authority(
            &pool,
            repository.id.as_uuid(),
            &runtime_commit,
            user_id,
            temporary.path().join("runtime-git-handoff"),
            None,
            time::Duration::minutes(10),
        )
        .await;
    let (expired_session_id, _, expired_credential, _) = seed_runtime_transport_authority(
        &pool,
        repository.id.as_uuid(),
        &runtime_commit,
        user_id,
        temporary.path().join("expired-runtime-git-handoff"),
        Some(runtime_binding_id),
        time::Duration::seconds(1),
    )
    .await;
    let (revoked_session_id, _, revoked_credential, _) = seed_runtime_transport_authority(
        &pool,
        repository.id.as_uuid(),
        &runtime_commit,
        user_id,
        temporary.path().join("revoked-runtime-git-handoff"),
        Some(runtime_binding_id),
        time::Duration::minutes(10),
    )
    .await;
    // Prerequisite: `cargo build -p git-http --bin pre-receive`, unless
    // HEPHAESTUS_GIT_RECEIVE_HOOK points to an absolute executable hook.
    let hook = runtime_receive_hook_path();
    let runtime_token = runtime_credential.expose_token().to_string();
    let runtime_credential_header = format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("heph-runtime:{runtime_token}"))
    );
    let runtime_authenticator = Arc::new(RuntimeGitHttpAuthenticator::new(Arc::new(
        PgRuntimeGitCredentialRepository::new(pool.clone()),
    )));
    runtime_authenticator
        .authenticate_git(
            Some(&runtime_credential_header),
            RequestId::new(),
            repository.id,
            GitOperation::Push,
        )
        .await
        .expect("direct production runtime Git authentication");
    let runtime_router = GitHttpService::new(
        Arc::clone(&repository_service),
        Arc::clone(&storage),
        runtime_authenticator,
        authorizer.clone(),
        backend,
        GitHttpLimits::default(),
    )
    .expect("runtime Git HTTP configuration")
    .with_runtime_receive_hook(hook)
    .expect("runtime receive hook configuration")
    .router();
    let runtime_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("runtime listener");
    let runtime_address = runtime_listener
        .local_addr()
        .expect("runtime listener address");
    let runtime_server = tokio::spawn(async move {
        axum::serve(runtime_listener, runtime_router)
            .await
            .expect("runtime smart HTTP server");
    });
    let runtime_remote = format!("http://{runtime_address}/{}", repository.id);
    git(&source, &["remote", "add", "runtime", &runtime_remote]).await;
    let other_runtime_remote = format!("http://{runtime_address}/{}", other_repository.id);
    git(
        &source,
        &["remote", "add", "other-runtime", &other_runtime_remote],
    )
    .await;
    let wrong_credential = format!(
        "Basic {}",
        BASE64_STANDARD.encode("heph-runtime:heph_git_v1_invalid")
    );
    let wrong_token = git_authenticated_result(
        &source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &wrong_credential,
    )
    .await;
    assert!(
        !wrong_token.status.success(),
        "wrong runtime token was accepted"
    );
    assert!(!String::from_utf8_lossy(&wrong_token.stderr).contains("heph_git_v1_invalid"));
    let other_repository_attempt = git_authenticated_result(
        &source,
        &["push", "other-runtime", "HEAD:refs/heads/main"],
        &runtime_credential_header,
    )
    .await;
    assert!(
        !other_repository_attempt.status.success(),
        "a runtime credential was accepted for another repository"
    );
    git_authenticated(
        &source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &runtime_credential_header,
    )
    .await;
    tokio::fs::write(source.join("outside.txt"), "denied\n")
        .await
        .expect("denied runtime change");
    git(&source, &["add", "outside.txt"]).await;
    git(&source, &["commit", "-m", "denied runtime path"]).await;
    let denied_path = git_authenticated_result(
        &source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &runtime_credential_header,
    )
    .await;
    assert!(
        !denied_path.status.success(),
        "out-of-scope runtime path was accepted"
    );
    assert!(
        String::from_utf8_lossy(&denied_path.stderr).contains("runtime receive denied"),
        "guarded hook denial was not reported"
    );
    git(&source, &["reset", "--hard", &runtime_commit]).await;
    tokio::fs::create_dir_all(source.join("sessions"))
        .await
        .expect("runtime ref fixture directory");
    tokio::fs::write(source.join("sessions/ref-denied.json"), "denied ref\n")
        .await
        .expect("denied runtime ref change");
    git(&source, &["add", "sessions/ref-denied.json"]).await;
    git(&source, &["commit", "-m", "denied runtime ref"]).await;
    let denied_ref = git_authenticated_result(
        &source,
        &["push", "runtime", "HEAD:refs/heads/other"],
        &runtime_credential_header,
    )
    .await;
    assert!(
        !denied_ref.status.success(),
        "out-of-scope runtime ref was accepted"
    );
    assert!(
        String::from_utf8_lossy(&denied_ref.stderr).contains("runtime receive denied"),
        "ref denial was not reported"
    );
    git(&source, &["reset", "--hard", &runtime_commit]).await;
    let denied_delete = git_authenticated_result(
        &source,
        &["push", "runtime", ":refs/heads/main"],
        &runtime_credential_header,
    )
    .await;
    assert!(
        !denied_delete.status.success(),
        "runtime branch deletion was accepted"
    );
    let baseline_tree = git_output(
        &source,
        &["rev-parse", &format!("{runtime_commit}^{{tree}}")],
    )
    .await;
    let force_commit = git_output(
        &source,
        &[
            "commit-tree",
            &baseline_tree,
            "-m",
            "non-fast-forward runtime change",
        ],
    )
    .await;
    let denied_force = git_authenticated_result(
        &source,
        &[
            "push",
            "runtime",
            &format!("+{force_commit}:refs/heads/main"),
        ],
        &runtime_credential_header,
    )
    .await;
    assert!(
        !denied_force.status.success(),
        "runtime force push was accepted"
    );
    assert!(
        String::from_utf8_lossy(&denied_force.stderr).contains("runtime receive denied"),
        "force-push denial was not reported by the receive hook"
    );
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    tokio::fs::create_dir_all(source.join("sessions"))
        .await
        .expect("expired runtime fixture directory");
    let expired_state: (String, bool) = sqlx::query_as(
        "SELECT status, expires_at < now()
           FROM runtime_authority_sessions WHERE id = $1",
    )
    .bind(expired_session_id)
    .fetch_one(&pool)
    .await
    .expect("inspect expired runtime Git session");
    assert_eq!(expired_state, (String::from("active"), true));
    tokio::fs::write(source.join("sessions/expired.json"), "expired\n")
        .await
        .expect("expired runtime change");
    git(&source, &["add", "sessions/expired.json"]).await;
    git(&source, &["commit", "-m", "expired runtime session"]).await;
    let expired_header = credential_header(&expired_credential);
    let denied_expired = git_authenticated_result(
        &source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &expired_header,
    )
    .await;
    assert!(
        !denied_expired.status.success(),
        "expired runtime session was accepted"
    );
    git(&source, &["reset", "--hard", &runtime_commit]).await;
    tokio::fs::create_dir_all(source.join("sessions"))
        .await
        .expect("revoked runtime fixture directory");
    tokio::fs::write(source.join("sessions/revoked.json"), "revoked\n")
        .await
        .expect("revoked runtime change");
    git(&source, &["add", "sessions/revoked.json"]).await;
    git(&source, &["commit", "-m", "revoked runtime session"]).await;
    let revoked_header = credential_header(&revoked_credential);
    sqlx::query(
        "UPDATE runtime_authority_sessions
            SET status = 'revoked', revoked_at = now(), revocation_reason = 'test'
          WHERE id = $1",
    )
    .bind(revoked_session_id)
    .execute(&pool)
    .await
    .expect("revoke runtime Git session");
    let denied_revoked = git_authenticated_result(
        &source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &revoked_header,
    )
    .await;
    assert!(
        !denied_revoked.status.success(),
        "revoked runtime session was accepted"
    );
    assert_eq!(
        git_output(
            &storage.repository_path(repository.id),
            &["rev-parse", "refs/heads/main"],
        )
        .await,
        runtime_commit,
        "guarded rejection changed canonical ref"
    );
    let other_receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(other_repository.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("other repository receive count");
    assert_eq!(
        other_receive_count, 0,
        "wrong-repository attempt reached receive persistence"
    );
    let runtime_receive_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives
          WHERE runtime_session_id = ANY($1)",
    )
    .bind(vec![
        runtime_session_id,
        expired_session_id,
        revoked_session_id,
    ])
    .fetch_one(&pool)
    .await
    .expect("runtime receive count");
    assert_eq!(
        runtime_receive_count, 1,
        "a rejected runtime receive was persisted"
    );
    let runtime_run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests
          WHERE receive_id IN (
              SELECT id FROM git_receives WHERE runtime_session_id = ANY($1)
          )",
    )
    .bind(vec![
        runtime_session_id,
        expired_session_id,
        revoked_session_id,
    ])
    .fetch_one(&pool)
    .await
    .expect("runtime receive run-request count");
    assert_eq!(
        runtime_run_request_count, 0,
        "runtime receive recursively triggered a run"
    );
    let runtime_receive: (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT runtime_session_id, runtime_attachment_id
         FROM git_receives
         WHERE repository_id = $1 AND runtime_session_id = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(runtime_session_id)
    .fetch_one(&pool)
    .await
    .expect("runtime transport receive provenance");
    assert_eq!(
        runtime_receive,
        (runtime_session_id, Some(origin_attachment))
    );
    let runtime_run_requests: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests
         WHERE receive_id = (
             SELECT id FROM git_receives WHERE runtime_session_id = $1
         )",
    )
    .bind(runtime_session_id)
    .fetch_one(&pool)
    .await
    .expect("runtime transport run requests");
    assert_eq!(runtime_run_requests, 0);
    runtime_server.abort();

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
        "SELECT commit_sha FROM agent_config_revisions
         WHERE repository_id = $1
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
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
           AND aggregate_id IN (
               SELECT run_id FROM run_requests WHERE repository_id = $2
           )",
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

    assert_eq!(receive_count, 2);
    assert_eq!(update_commit, commit);
    assert_eq!(revision_commit, runtime_commit);
    assert_eq!(run_request_count, 2);
    assert_eq!(start_event_count, 2);
    assert_eq!(run_authorization_count, 2);
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

// Keeping the full immutable graph together makes this compatibility-breaking
// exact-trigger fixture easier to audit than a chain of partially valid rows.
#[allow(clippy::too_many_lines)]
async fn seed_attached_instance(
    pool: &PgPool,
    user_id: UserId,
    project_id: forge_domain::ProjectId,
    repository_id: forge_domain::RepositoryId,
    commit: &str,
) {
    let build_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin exact instance fixture");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, created_by, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5, now())",
    )
    .bind(build_id)
    .bind(repository_id.as_uuid())
    .bind(commit)
    .bind([1_u8; 32].as_slice())
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'reviewer')",
    )
    .bind(family_id)
    .bind(repository_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact family");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state,
          publication_actor_id, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                 '{}', $6, $7, 'published', $8, now())",
    )
    .bind(release_id)
    .bind(repository_id.as_uuid())
    .bind(commit)
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'reviewer', 'Reviewer', $4, $5,
                 '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(json!({
        "command": "bin/reviewer",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "fixture"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(&mut *tx)
    .await
    .expect("seed exact release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state, created_by)
         VALUES ($1, $2, $3, $4, 'active', $5)",
    )
    .bind(instance_id)
    .bind(project_id.as_uuid())
    .bind(family_id)
    .bind(format!("reviewer_{}", Uuid::new_v4().simple()))
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable,
          diagnostics, created_by)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $5, $7,
                 'platform/test', true, '[]', $8)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 256, "network": "disabled"}))
    .bind(json!({"network": "disabled"}))
    .bind([6_u8; 32].as_slice())
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(&mut *tx)
        .await
        .expect("activate exact revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy, enabled, created_by)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(project_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact attachment");
    tx.commit().await.expect("commit exact instance fixture");
}

fn runtime_receive_binding_hash(repository_id: Uuid) -> [u8; 32] {
    let ceiling = GitCapabilityCeiling::new(GitCapabilityCeilingInput {
        operations: vec![CapabilityGitOperation::Receive],
        ref_globs: vec![
            RefGlob::parse_explicitly_broad("refs/heads/main").expect("runtime ref glob"),
        ],
        changed_path_globs: vec![
            ChangedPathGlob::parse_explicitly_broad("runtime.txt").expect("runtime path glob"),
        ],
        update_policy: RefUpdatePolicy {
            branches: BranchRefPolicy {
                updates: BranchUpdatePolicy::FastForwardOnly,
                create: RefMutationPermission::Allow,
                delete: RefMutationPermission::Deny,
            },
            tags: RefNamespacePolicy::default(),
            other: RefNamespacePolicy::default(),
        },
        transfer_limits: TransferLimits::new(16_777_216, 1_073_741_824, 1_000_000, 256)
            .expect("runtime transfer limits"),
        exact_parent_required: false,
    })
    .expect("runtime capability ceiling");
    *BoundGitCapability::new(
        CapabilityRepositoryId::new(repository_id),
        ceiling.clone(),
        &ceiling,
    )
    .expect("runtime bound capability")
    .normalized_hash()
    .expect("runtime capability hash")
    .as_bytes()
}

fn credential_header(credential: &RuntimeGitCredential) -> String {
    let token = credential.expose_token().to_string();
    format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("heph-runtime:{token}"))
    )
}

// The transport test uses the production Git HTTP/authentication path while
// constructing only the already-published runtime join graph that this crate
// does not own. Release-authority integration tests cover creation of these
// immutable rows through the runtime authority adapter.
#[allow(clippy::too_many_lines)]
async fn seed_runtime_transport_authority(
    pool: &PgPool,
    repository_id: Uuid,
    commit: &str,
    user_id: UserId,
    handoff_root: std::path::PathBuf,
    existing_binding_id: Option<Uuid>,
    lifetime: time::Duration,
) -> (Uuid, Uuid, RuntimeGitCredential, Uuid) {
    let (attachment_id, instance_id): (Uuid, Uuid) =
        sqlx::query_as("SELECT id, instance_id FROM agent_attachments WHERE repository_id = $1")
            .bind(repository_id)
            .fetch_one(pool)
            .await
            .expect("runtime transport attachment");
    let (revision_id, release_id, release_agent_id): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT revision.id, release.id, release_agent.id
         FROM agent_instance_revisions AS revision
         JOIN release_agents AS release_agent ON release_agent.id = revision.release_agent_id
         JOIN releases AS release ON release.id = release_agent.release_id
         WHERE revision.instance_id = $1",
    )
    .bind(instance_id)
    .fetch_one(pool)
    .await
    .expect("runtime transport revision");
    let run_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let runtime_session_id = Uuid::new_v4();
    let binding_id = existing_binding_id.unwrap_or_else(Uuid::new_v4);
    let now = time::OffsetDateTime::now_utc();
    let expires_at = now + lifetime;
    let binding_hash = runtime_receive_binding_hash(repository_id);
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, state, created_at, updated_at,
          instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, requires_state)
         VALUES ($1, $2, 'queued', now(), now(), $3, $4, $5, $6, $7,
                 'normal', false)",
    )
    .bind(run_id)
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(attachment_id)
    .execute(pool)
    .await
    .expect("runtime transport run");
    sqlx::query(
        "INSERT INTO run_authorization_snapshots
         (id, run_id, instance_id, instance_revision_id,
          authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'test/v1', $5)",
    )
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime transport snapshot");
    let session_credential_hash =
        [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    sqlx::query(
        "INSERT INTO runtime_authority_sessions
         (id, snapshot_id, run_id, instance_id, instance_revision_id,
          attachment_id, identity_hash, snapshot_hash, issuance_generation,
          credential_hash, status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9, 'pending_handoff',
                 $10, $11, NULL)",
    )
    .bind(runtime_session_id)
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(attachment_id)
    .bind([2_u8; 32].as_slice())
    .bind([1_u8; 32].as_slice())
    .bind(session_credential_hash.as_slice())
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("runtime transport session");
    let mut transaction = pool.begin().await.expect("begin runtime transport graph");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *transaction)
        .await
        .expect("enable runtime transport fixture mode");
    if existing_binding_id.is_none() {
        sqlx::query(
            "INSERT INTO agent_capability_bindings
             (id, instance_revision_id, release_agent_id, requirement_id,
              requirement_hash, slot_key, resource_kind, resource_id,
              granted_operations, normalized_hash, authorization_model_version,
              created_by)
             VALUES ($1, $2, $3, $4, $5, 'content', 'repository', $6,
                     ARRAY['update_ref'], $7, 'test/v1', $8)",
        )
        .bind(binding_id)
        .bind(revision_id)
        .bind(release_agent_id)
        .bind(Uuid::new_v4())
        .bind([3_u8; 32].as_slice())
        .bind(repository_id)
        .bind(binding_hash.as_slice())
        .bind(user_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("runtime generic capability binding");
    }
    sqlx::query(
        "INSERT INTO run_authorization_snapshot_bindings
         (snapshot_id, instance_revision_id, ordinal, binding_id,
          binding_hash, slot_key, resource_kind, resource_id,
          granted_operations)
         VALUES ($1, $2, 0, $3, $4, 'content', 'repository', $5,
                 ARRAY['update_ref'])",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(binding_id)
    .bind(binding_hash.as_slice())
    .bind(repository_id)
    .execute(&mut *transaction)
    .await
    .expect("runtime snapshot capability binding");
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
         (snapshot_id, instance_revision_id, binding_id, repository_id,
          grammar_version, git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, expected_parent, normalized_hash)
         VALUES ($1, $2, $3, $4, 1, ARRAY['receive'],
                 ARRAY['refs/heads/main'], ARRAY['runtime.txt'], 'fast_forward_only',
                 true, false, false, false, false, false, false, false,
                 16777216, 1073741824, 1000000, 256, false, NULL, $5)",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(binding_id)
    .bind(repository_id)
    .bind(binding_hash.as_slice())
    .execute(&mut *transaction)
    .await
    .expect("runtime transport Git snapshot");
    sqlx::query(
        "INSERT INTO run_instance_provenance
         (run_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, target_repository_id, target_ref,
          target_commit, parameter_hash, platform_policy_version, phase,
          authorization_model_version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'refs/heads/main', $8, $9,
                 'platform/v1', 'normal', 'test/v1')",
    )
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(attachment_id)
    .bind(repository_id)
    .bind(commit)
    .bind([8_u8; 32].as_slice())
    .execute(&mut *transaction)
    .await
    .expect("runtime transport provenance");
    transaction
        .commit()
        .await
        .expect("commit runtime transport graph");
    let handoff = EncryptedFileRuntimeGitHandoffStore::new(handoff_root, [0x11; 32])
        .expect("runtime Git handoff store");
    let credential_issuer = RuntimeGitCredentialIssuer::new(
        PgRuntimeGitCredentialRepository::new(pool.clone()),
        handoff,
    );
    let issued_credential = credential_issuer
        .issue(
            RuntimeSessionId::from_uuid(runtime_session_id),
            RuntimeCredentialGeneration::INITIAL,
            expires_at,
            now,
        )
        .await
        .expect("issue runtime Git credential");
    sqlx::query(
        "UPDATE runtime_authority_sessions
         SET status = 'active', acknowledged_at = $2
         WHERE id = $1",
    )
    .bind(runtime_session_id)
    .bind(now)
    .execute(pool)
    .await
    .expect("activate runtime Git session");
    let permission: i32 = sqlx::query_scalar(
        "SELECT check_permission(
             'agent_instance', $1, 'agent_update_ref', 'repository', $2
         )",
    )
    .bind(instance_id.to_string())
    .bind(repository_id.to_string())
    .fetch_one(pool)
    .await
    .expect("runtime capability permission");
    assert_eq!(permission, 1, "runtime capability permission denied");
    let authenticated: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authenticate_runtime_git_credential($1, $2, 'receive')",
    )
    .bind(
        issued_credential
            .credential
            .storage_hash()
            .as_bytes()
            .as_slice(),
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("runtime credential authentication query");
    assert_eq!(authenticated, 1, "production runtime credential rejected");
    (
        runtime_session_id,
        attachment_id,
        issued_credential.credential,
        binding_id,
    )
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

fn runtime_receive_hook_path() -> PathBuf {
    let hook = std::env::var_os("HEPHAESTUS_GIT_RECEIVE_HOOK").map_or_else(
        || {
            let test_executable =
                std::env::current_exe().expect("locate the active smart HTTP test executable");
            test_executable
                .parent()
                .and_then(Path::parent)
                .map(|profile| profile.join("pre-receive"))
                .expect("locate the active Cargo target profile")
        },
        PathBuf::from,
    );
    assert!(
        hook.is_absolute(),
        "runtime receive hook must be an absolute path: {}",
        hook.display()
    );
    assert_eq!(
        hook.file_name().and_then(|name| name.to_str()),
        Some("pre-receive"),
        "runtime receive hook must be named pre-receive: {}",
        hook.display()
    );
    assert!(
        hook.is_file(),
        "runtime receive hook is missing at {}; build it with `cargo build -p git-http --bin pre-receive` or set HEPHAESTUS_GIT_RECEIVE_HOOK",
        hook.display()
    );
    assert!(
        runtime_hook_is_executable(&hook),
        "runtime receive hook is not executable: {}",
        hook.display()
    );
    hook.canonicalize().unwrap_or(hook)
}

#[cfg(unix)]
fn runtime_hook_is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn runtime_hook_is_executable(path: &Path) -> bool {
    path.is_file()
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

async fn git_authenticated(directory: &Path, arguments: &[&str], authorization: &str) {
    let output = git_authenticated_result(directory, arguments, authorization).await;
    assert!(
        output.status.success(),
        "authenticated git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn git_authenticated_result(
    directory: &Path,
    arguments: &[&str],
    authorization: &str,
) -> std::process::Output {
    Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: {authorization}"),
        )
        .output()
        .await
        .expect("run authenticated Git")
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

use super::super::{UiRepositoryGitState, router};
use async_trait::async_trait;
use authz_domain::{
    AuthorizationDecision, AuthzError, GitRepositoryAuthorizer, GitRepositoryOperation,
};
use axum::Router;
use forge_domain::RepositoryId;
use forge_postgres::PgForgeRepository;
use forge_service::GitStorage;
use git_http::{
    AuthenticationError, GitAuthenticator, GitHttpLimits, GitHttpService, PostgresGitAuthorizer,
    Principal,
};
use identity_domain::RequestId;
use release_domain::ui_browser::UiBrowserSessionSecret;
use release_service::{
    ActiveUiGenerationHost, UiNamespace, UiPublicPort, ui_browser_host::UiGenerationHost,
};
use release_service::{
    UiBrowserRepositoryGitAuthorization, UiGenerationHostResolver, UiGitAuthorizationError,
    UiHostLookupError, UiRepositoryGitAuthorization, UiRepositoryGitOperation,
};
use sqlx::postgres::PgPoolOptions;
use std::{path::PathBuf, process::Output, sync::Arc};

#[allow(dead_code)]
pub(super) mod resource_fixture {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../heph-core/forge/release/postgres/tests/support/ui_browser_resource_fixture.rs"
    ));
}

struct TestHostResolver {
    active_generation: Option<release_domain::UiInstallationGenerationId>,
}

#[async_trait]
impl UiGenerationHostResolver for TestHostResolver {
    async fn resolve_active_generation_host(
        &self,
        host: UiGenerationHost,
    ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError> {
        Ok(self
            .active_generation
            .filter(|generation| *generation == host.generation_id())
            .map(|generation_id| ActiveUiGenerationHost { generation_id }))
    }
}

struct TestGitAuthority;

#[async_trait]
impl UiBrowserRepositoryGitAuthorization for TestGitAuthority {
    async fn authorize_repository_git(
        &self,
        _request_id: RequestId,
        _session_secret: UiBrowserSessionSecret,
        _expected_generation_id: release_domain::UiInstallationGenerationId,
        _repository_id: RepositoryId,
        _operation: UiRepositoryGitOperation,
    ) -> Result<UiRepositoryGitAuthorization, UiGitAuthorizationError> {
        Err(UiGitAuthorizationError::Unauthorized)
    }
}

pub(super) struct TestAuthenticator;

#[async_trait]
impl GitAuthenticator for TestAuthenticator {
    async fn authenticate(
        &self,
        _credential: Option<&str>,
        _request_id: RequestId,
    ) -> Result<Principal, AuthenticationError> {
        Err(AuthenticationError::denied("test"))
    }
}

struct AllowGit;

#[async_trait]
impl GitRepositoryAuthorizer for AllowGit {
    async fn authorize_git(
        &self,
        _repository_id: uuid::Uuid,
        _operation: GitRepositoryOperation,
        _identity: &identity_domain::AuthenticatedIdentity,
    ) -> Result<AuthorizationDecision, AuthzError> {
        Ok(AuthorizationDecision::Allow)
    }
}

pub(super) async fn test_router() -> Router {
    test_router_with_host(None).await
}

pub(super) async fn test_router_with_host(
    active_generation: Option<release_domain::UiInstallationGenerationId>,
) -> Router {
    let root = tempfile::tempdir().expect("Git root");
    let storage = Arc::new(GitStorage::initialize(root.path()).await.expect("storage"));
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://test:test@127.0.0.1:1/test")
        .expect("lazy pool");
    let repository = Arc::new(PgForgeRepository::new(pool, Arc::clone(&storage)));
    let authorizer = Arc::new(PostgresGitAuthorizer::new(Arc::new(AllowGit)));
    let git = Arc::new(
        GitHttpService::new(
            repository,
            storage,
            Arc::new(TestAuthenticator),
            authorizer,
            PathBuf::from("/bin/false"),
            GitHttpLimits::default(),
        )
        .expect("Git service"),
    );
    let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
    let state = Arc::new(UiRepositoryGitState::new(
        Arc::new(TestHostResolver { active_generation }),
        Arc::new(TestGitAuthority),
        git,
        namespace,
        UiPublicPort::https_default(),
        Arc::new(crate::ui_audit::NoopAuditSink),
    ));
    router(state)
}

pub(super) async fn run_git(directory: &std::path::Path, args: &[&str]) {
    let result = tokio::process::Command::new("git")
        .args(args)
        .current_dir(directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .await
        .expect("spawn git");
    assert!(result.status.success(), "git failed: {result:?}");
}

pub(super) async fn run_git_owned(directory: &std::path::Path, args: Vec<String>) -> Output {
    tokio::process::Command::new("git")
        .args(&args)
        .current_dir(directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .await
        .expect("spawn git")
}

pub(super) async fn git_backend_path() -> PathBuf {
    let output = tokio::process::Command::new("git")
        .args(["--exec-path"])
        .output()
        .await
        .expect("git exec path");
    assert!(output.status.success());
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("git exec path UTF-8")
            .trim(),
    )
    .join("git-http-backend")
}

pub(super) async fn role_pool(database_url: &str, role: &str) -> sqlx::PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect({
            let role = role.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role)
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect role pool")
}

pub(super) async fn insert_repository_child(
    pool: &sqlx::PgPool,
    fixture: &resource_fixture::Fixture,
    secret: [u8; 32],
) {
    let handoff = uuid::Uuid::new_v4();
    let mut transaction = pool.begin().await.expect("begin repository child");
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'schema-repository',
                 statement_timestamp(), statement_timestamp() + interval '60 seconds')",
    )
    .bind(handoff)
    .bind(secret.to_vec())
    .bind(uuid::Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(fixture.repository_installation)
    .bind(fixture.repository_generation)
    .bind(fixture.organization)
    .execute(&mut *transaction)
    .await
    .expect("insert repository handoff");
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         SELECT $1, $2, $3, id, parent_session_id, installation_id,
                generation_id, organization_id, route, issued_at,
                statement_timestamp() + interval '1 hour'
         FROM ui_browser_handoffs WHERE id = $4",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(
        UiBrowserSessionSecret::from_bytes(secret)
            .digest()
            .as_bytes()
            .to_vec(),
    )
    .bind(uuid::Uuid::new_v4())
    .bind(handoff)
    .execute(&mut *transaction)
    .await
    .expect("insert repository child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *transaction)
        .await
        .expect("consume repository handoff");
    transaction.commit().await.expect("commit repository child");
}

use super::*;

pub(super) async fn initialize(
    pool: PgPool,
) -> (tempfile::TempDir, Arc<GitStorage>, Arc<PgForgeRepository>) {
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
        latest_migration, 100,
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
    (temporary, storage, repository_service)
}

pub(super) async fn seed_repository_graph(
    pool: &PgPool,
    repository_service: &Arc<PgForgeRepository>,
) -> (
    UserId,
    AuthenticatedIdentity,
    forge_domain::Project,
    forge_domain::Repository,
    forge_domain::Repository,
) {
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
        .execute(pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind("smart-http-integration")
        .execute(pool)
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
    .execute(pool)
    .await
    .expect("organization owner");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.id.as_uuid())
        .bind(user_id.as_uuid())
        .execute(pool)
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
    (user_id, identity, project, repository, other_repository)
}

pub(super) async fn start_http(
    pool: &PgPool,
    storage: &Arc<GitStorage>,
    repository_service: &Arc<PgForgeRepository>,
    repository: &forge_domain::Repository,
    identity: &AuthenticatedIdentity,
) -> (
    Arc<RecordingAuthorizer>,
    String,
    PathBuf,
    String,
    tokio::task::JoinHandle<()>,
) {
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
        Arc::clone(repository_service),
        Arc::clone(storage),
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
    (authorizer, basic_credential, backend, remote, server)
}

pub(super) async fn seed_git(
    pool: &PgPool,
    temporary: &tempfile::TempDir,
    project: &forge_domain::Project,
    repository: &forge_domain::Repository,
    user_id: UserId,
    basic_credential: &str,
    remote: &str,
) -> (PathBuf, String, String) {
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
    seed_attached_instance(pool, user_id, project.id, repository.id, &commit).await;
    let instance_id: Uuid =
        sqlx::query_scalar("SELECT instance_id FROM agent_attachments WHERE repository_id = $1")
            .bind(repository.id.as_uuid())
            .fetch_one(pool)
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
    .execute(pool)
    .await
    .expect("transport sibling attachment");
    git(&source, &["remote", "add", "origin", remote]).await;
    git_authenticated(
        &source,
        &["push", "origin", "HEAD:refs/heads/main"],
        basic_credential,
    )
    .await;

    let clone = temporary.path().join("clone");
    git_authenticated(
        temporary.path(),
        &["clone", remote, clone.to_str().expect("UTF-8 clone path")],
        basic_credential,
    )
    .await;
    git_authenticated(&clone, &["fetch", "origin"], basic_credential).await;

    tokio::fs::write(source.join("runtime.txt"), "runtime receive\n")
        .await
        .expect("runtime change");
    git(&source, &["add", "runtime.txt"]).await;
    git(&source, &["commit", "-m", "runtime receive"]).await;
    let runtime_commit = git_output(&source, &["rev-parse", "HEAD"]).await;
    (source, commit, runtime_commit)
}

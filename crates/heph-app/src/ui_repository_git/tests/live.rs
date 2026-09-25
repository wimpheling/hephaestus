use super::super::{UiRepositoryGitState, router};
use super::support::{
    TestAuthenticator, git_backend_path, insert_repository_child, resource_fixture, role_pool,
    run_git, run_git_owned,
};
use authz_postgres::PostgresMelangeAuthorizer;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use forge_domain::RepositoryId;
use forge_postgres::PgForgeRepository;
use forge_service::GitStorage;
use git_http::{GitHttpLimits, GitHttpService, PostgresGitAuthorizer};
use release_service::{
    UiGenerationHostResolver, UiNamespace, UiPublicPort,
    ui_browser_host::{UI_CHILD_COOKIE, UiGenerationHost},
};
use sqlx::postgres::PgPoolOptions;
use std::{env, net::SocketAddr, sync::Arc};
use uuid::Uuid;

#[tokio::test]
#[serial_test::serial]
async fn real_child_fetch_and_push_route_persists_human_receive_and_audit() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping live UI Git route test: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0100");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app_pool = role_pool(&database_url, "hephaestus_app").await;
    let fixture = resource_fixture::seed_fixture_reusing_installation_helpers(&worker).await;
    let secret = resource_fixture::fixture_session_secret(fixture.actor, 77);
    // The shared UI fixture intentionally leaves repository write authority
    // absent so its revocation matrix remains meaningful. This production
    // route proof needs an explicit live human CanWrite grant.
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(fixture.source_project)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed route-test repository maintainer");
    insert_repository_child(&worker, &fixture, secret).await;

    let temporary = tempfile::tempdir().expect("temporary Git root");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("Git storage"),
    );
    let repository_id = RepositoryId::from_uuid(fixture.repository);
    storage
        .create_bare(repository_id, "main")
        .await
        .expect("create canonical bare repository");
    let source = temporary.path().join("source");
    run_git(
        temporary.path(),
        &["init", "--initial-branch=main", source.to_str().unwrap()],
    )
    .await;
    run_git(&source, &["config", "user.name", "UI Test"]).await;
    run_git(
        &source,
        &["config", "user.email", "ui-test@example.invalid"],
    )
    .await;
    tokio::fs::write(source.join("README.md"), "initial\n")
        .await
        .expect("write initial source");
    run_git(&source, &["add", "."]).await;
    run_git(&source, &["commit", "-m", "initial"]).await;
    run_git(
        &source,
        &[
            "push",
            storage.repository_path(repository_id).to_str().unwrap(),
            "main",
        ],
    )
    .await;

    let backend = git_backend_path().await;
    let forge = Arc::new(
        PgForgeRepository::new(worker.clone(), Arc::clone(&storage))
            .with_authorizer(Arc::new(PostgresMelangeAuthorizer)),
    );
    let git = Arc::new(
        GitHttpService::new(
            forge,
            Arc::clone(&storage),
            Arc::new(TestAuthenticator),
            Arc::new(PostgresGitAuthorizer::new(Arc::new(
                authz_postgres::PostgresGitAuthorizer::new(worker.clone()),
            ))),
            backend,
            GitHttpLimits::default(),
        )
        .expect("Git HTTP service"),
    );
    let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
    let port = UiPublicPort::https_default();
    let host = UiGenerationHost::from_generation_id(
        release_domain::UiInstallationGenerationId::from_uuid(fixture.repository_generation),
    );
    let authority = host.authority(&namespace, port);
    let host_resolver: Arc<dyn UiGenerationHostResolver> = Arc::new(
        release_postgres::PgUiGenerationHostResolver::new(app_pool.clone()),
    );
    let serving_store = Arc::new(release_postgres::PgUiBrowserServingStore::new(
        app_pool.clone(),
    ));
    let audit_sink: Arc<dyn release_service::UiRequestAuditSink> = Arc::new(
        release_postgres::PgUiRequestAuditRepository::new(worker.clone()),
    );
    let state = Arc::new(UiRepositoryGitState::new(
        host_resolver.clone(),
        serving_store.clone(),
        Arc::clone(&git),
        namespace.clone(),
        port,
        audit_sink.clone(),
    ));
    let context = Arc::new(crate::ui_context::UiContextState::new(
        host_resolver,
        serving_store,
        namespace.clone(),
        port,
        audit_sink,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind UI Git test listener");
    let address = listener.local_addr().expect("UI Git listener address");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(state)
                .merge(crate::ui_context::router(context))
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("UI Git server");
    });
    let remote = format!("http://{address}/_heph/git/{repository_id}");
    let cookie = format!("{UI_CHILD_COOKIE}={}", URL_SAFE_NO_PAD.encode(secret));
    let context_result = tokio::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "-H",
            &format!("Host: {authority}"),
            "-H",
            &format!("Cookie: {cookie}"),
            &format!("http://{address}/_heph/ui-context"),
        ])
        .output()
        .await
        .expect("spawn UI context curl");
    assert!(
        context_result.status.success(),
        "UI context request failed: {context_result:?}"
    );
    let context: serde_json::Value =
        serde_json::from_slice(&context_result.stdout).expect("UI context JSON");
    assert_eq!(context["repository_id"], fixture.repository.to_string());
    assert_eq!(context.as_object().expect("UI context object").len(), 1);
    let common = vec![
        String::from("-c"),
        format!("http.extraHeader=Host: {authority}"),
        String::from("-c"),
        format!("http.extraHeader=Cookie: {cookie}"),
        String::from("-c"),
        format!("http.extraHeader=Origin: https://{authority}"),
    ];
    let clone_path = temporary.path().join("clone");
    let mut clone_args = common.clone();
    clone_args.extend([
        String::from("clone"),
        remote.clone(),
        clone_path.to_str().unwrap().to_owned(),
    ]);
    let clone_result = run_git_owned(temporary.path(), clone_args).await;
    assert!(
        clone_result.status.success(),
        "UI Git clone failed: {clone_result:?}"
    );
    assert_eq!(
        tokio::fs::read_to_string(clone_path.join("README.md"))
            .await
            .expect("cloned README"),
        "initial\n"
    );
    run_git(&clone_path, &["config", "user.name", "UI Test"]).await;
    run_git(
        &clone_path,
        &["config", "user.email", "ui-test@example.invalid"],
    )
    .await;
    tokio::fs::write(clone_path.join("README.md"), "pushed\n")
        .await
        .expect("write pushed source");
    run_git(&clone_path, &["add", "."]).await;
    run_git(&clone_path, &["commit", "-m", "browser push"]).await;
    let mut push_args = common;
    push_args.extend([String::from("push"), remote, String::from("HEAD:main")]);
    let push_result = run_git_owned(&clone_path, push_args).await;
    run_git(
        storage
            .repository_path(repository_id)
            .parent()
            .expect("bare parent"),
        &[
            "--git-dir",
            storage.repository_path(repository_id).to_str().unwrap(),
            "rev-parse",
            "refs/heads/main",
        ],
    )
    .await;
    assert!(
        push_result.status.success(),
        "UI Git push failed: {push_result:?}"
    );
    let receive: (Uuid, Uuid, String, Uuid) = sqlx::query_as(
        "SELECT id, actor_id, principal, request_id
         FROM git_receives
         WHERE repository_id = $1 AND principal = $2
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(fixture.repository)
    .bind(format!("user:{}", fixture.actor))
    .fetch_one(&bootstrap)
    .await
    .expect("durable human receive");
    assert_eq!(receive.1, fixture.actor);
    assert_eq!(receive.2, format!("user:{}", fixture.actor));
    let audit: (String, String, Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT decision, outcome, actor_id, installation_id, generation_id
         FROM ui_request_audit_events
         WHERE request_id = $1",
    )
    .bind(receive.3)
    .fetch_one(&bootstrap)
    .await
    .expect("durable UI Git audit event");
    assert_eq!(audit.0, "allowed");
    assert_eq!(audit.1, "succeeded");
    assert_eq!(audit.2, fixture.actor);
    assert_eq!(audit.3, fixture.repository_installation);
    assert_eq!(audit.4, fixture.repository_generation);
    server.abort();
}

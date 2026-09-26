use super::{
    CONFIG_TEMPLATE, INVALID_UI, VALID_UI,
    fixture::{
        assert_blocked_by, commit_source, decode_hash, named_app_pool, seed_catalog_images,
        seed_permissions, wait_for_lock,
    },
};
use authz_postgres::PostgresMelangeAuthorizer;
use control_plane_postgres::build::{BuildApplication, BuildError, RequestBuild};
use forge_domain::{GitRef, OrganizationId, ReceiveId};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn receive_commits_before_manual_request_observes_ui_capture() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping receive/manual UI ordering: test URL is unset");
        return;
    };

    run_ordering_case(&database_url, VALID_UI, true).await;
    run_ordering_case(&database_url, INVALID_UI, false).await;
}

// Keep both lock-order cases together so their barrier and authorization paths
// remain visibly identical; the fixture is intentionally integration-sized.
#[allow(clippy::too_many_lines)]
// Setup, lock observation, and result assertions are intentionally kept in
// one case so valid and invalid captures use the same ordering fixture.
#[allow(clippy::cognitive_complexity)]
async fn run_ordering_case(database_url: &str, ui_manifest: &str, valid: bool) {
    let admin = PgPoolOptions::new()
        .max_connections(8)
        .connect(database_url)
        .await
        .expect("connect ordering test database");
    let temporary = tempfile::tempdir().expect("ordering test temporary root");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("initialize ordering Git storage"),
    );
    let setup_service = PgForgeRepository::new(admin.clone(), Arc::clone(&storage));
    setup_service.initialize().await.expect("apply migrations");

    let organization = OrganizationId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization.as_uuid())
        .bind(format!("receive-ordering-{organization}"))
        .execute(&admin)
        .await
        .expect("seed organization");
    let project = setup_service
        .create_project_trusted(organization, "receive-ordering-project")
        .await
        .expect("seed project");
    let repository = setup_service
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("seed repository");
    seed_catalog_images(&admin, repository.id.as_uuid()).await;

    let owner = UserId::new();
    seed_permissions(
        &admin,
        owner,
        organization,
        project.id.as_uuid(),
        repository.id.as_uuid(),
    )
    .await;
    let identity = AuthenticatedIdentity::new(
        owner,
        "https://receive-manual-ordering.example",
        format!("receive-manual-{owner}"),
        json!({"email_verified": true}),
        RequestId::new(),
    );
    let config = CONFIG_TEMPLATE
        .replace(
            "__BUILD_IMAGE__",
            &format!("ordering-build-{}", repository.id.as_uuid().simple()),
        )
        .replace(
            "__RUNTIME_IMAGE__",
            &format!("ordering-runtime-{}", repository.id.as_uuid().simple()),
        );
    let (commit, update) = commit_source(&temporary, &repository, &config, ui_manifest).await;
    let parsed = agent_config::parse(config.as_bytes());
    let parsed_config = parsed.config.expect("valid agent configuration");
    let normalized_hash = decode_hash(
        parsed
            .normalized_hash
            .expect("normalized configuration hash")
            .as_str(),
    );
    let base_hash = agent_config::build_identity::base_build_definition_hash(
        parsed_config.build.as_ref().expect("build declaration"),
    )
    .expect("base build hash");

    let receive_name = format!(
        "receive-ordering-receive-{}",
        repository.id.as_uuid().simple()
    );
    let manual_name = format!(
        "receive-ordering-manual-{}",
        repository.id.as_uuid().simple()
    );
    let receive_pool = named_app_pool(database_url, &receive_name).await;
    let manual_pool = named_app_pool(database_url, &manual_name).await;
    let receive_service = PgForgeRepository::new(receive_pool.clone(), Arc::clone(&storage))
        .with_authorizer(Arc::new(PostgresMelangeAuthorizer));
    let application = BuildApplication::new(manual_pool.clone());

    let barrier_receive_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, principal, status, accepted_at, created_at)
         VALUES ($1, $2, 'ordering-barrier', 'accepted', now(), now())",
    )
    .bind(barrier_receive_id)
    .bind(repository.id.as_uuid())
    .execute(&admin)
    .await
    .expect("seed lock barrier receive");
    sqlx::query(
        "INSERT INTO git_refs
         (repository_id, git_ref, commit_sha, updated_by_receive_id)
         VALUES ($1, 'refs/heads/main', repeat('f', 40), $2)",
    )
    .bind(repository.id.as_uuid())
    .bind(barrier_receive_id)
    .execute(&admin)
    .await
    .expect("seed lock barrier ref");
    let mut barrier = admin.begin().await.expect("begin lock barrier");
    sqlx::query("SELECT set_config('application_name', $1, false)")
        .bind(format!(
            "receive-ordering-holder-{}",
            repository.id.as_uuid().simple()
        ))
        .execute(&mut *barrier)
        .await
        .expect("name lock barrier");
    let barrier_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *barrier)
        .await
        .expect("lock barrier pid");
    sqlx::query("SELECT git_ref FROM git_refs WHERE repository_id = $1 FOR UPDATE")
        .bind(repository.id.as_uuid())
        .execute(&mut *barrier)
        .await
        .expect("hold ref lock barrier");

    let receive_repository = repository.clone();
    let receive_identity = identity.clone();
    let receive_update = update.clone();
    let receive_task = tokio::spawn(async move {
        receive_service
            .accept_receive_as(
                &receive_repository,
                ReceiveId::new(),
                "receive-ordering",
                Some(&receive_identity),
                std::slice::from_ref(&receive_update),
            )
            .await
    });
    let receive_pid = wait_for_lock(&admin, &receive_name, "INSERT INTO git_refs").await;
    assert_blocked_by(&admin, receive_pid, barrier_pid).await;

    let manual_identity = identity.clone();
    let manual_request = RequestBuild {
        repository_id: repository.id.as_uuid(),
        source_commit: commit.as_str().to_owned(),
        build_definition_hash: base_hash,
        configuration_hash: normalized_hash,
    };
    let manual_task = tokio::spawn(async move {
        application
            .request_build(&manual_identity, manual_request)
            .await
    });
    let manual_pid = wait_for_lock(&admin, &manual_name, "FROM repositories").await;
    assert_blocked_by(&admin, manual_pid, receive_pid).await;
    barrier
        .commit()
        .await
        .expect("release receive lock barrier");

    let received = timeout(Duration::from_secs(10), receive_task)
        .await
        .expect("receive completion")
        .expect("receive task")
        .expect("accepted receive");
    let manual = timeout(Duration::from_secs(10), manual_task)
        .await
        .expect("manual request completion")
        .expect("manual request task");

    if valid {
        assert_eq!(received.build_requests.len(), 1);
        let received_build_id = received.build_requests[0].as_uuid();
        let manual = manual.expect("manual request observes valid capture");
        assert_eq!(manual.id, received_build_id);
        let stored_hash: Vec<u8> =
            sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
                .bind(received_build_id)
                .fetch_one(&admin)
                .await
                .expect("stored receive build hash");
        let ui_hash: Vec<u8> = sqlx::query_scalar(
            "SELECT normalized_ui_hash
             FROM ui_source_manifest_revisions
             WHERE repository_id = $1 AND source_commit = $2",
        )
        .bind(repository.id.as_uuid())
        .bind(commit.as_str())
        .fetch_one(&admin)
        .await
        .expect("stored valid UI hash");
        let ui_hash: [u8; 32] = ui_hash.try_into().expect("valid UI hash width");
        let expected_hash =
            agent_config::build_identity::ui_build_definition_hash(base_hash, ui_hash, None);
        assert_ne!(expected_hash, base_hash);
        assert_eq!(stored_hash, expected_hash.to_vec());
        let link_count: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM build_request_ui_source_manifests
             WHERE build_request_id = $1 AND source_status = 'valid'",
        )
        .bind(received_build_id)
        .fetch_one(&admin)
        .await
        .expect("valid UI build link");
        assert_eq!(link_count, 1);
        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox
             WHERE subject = 'hephaestus.build.requested.v1' AND aggregate_id = $1",
        )
        .bind(received_build_id)
        .fetch_one(&admin)
        .await
        .expect("valid build event count");
        assert_eq!(event_count, 1);
    } else {
        assert_eq!(received.invalid_configurations, 0);
        assert!(received.build_requests.is_empty());
        assert!(matches!(manual, Err(BuildError::FailedPrecondition)));
        let build_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM build_requests
             WHERE repository_id = $1 AND source_commit = $2",
        )
        .bind(repository.id.as_uuid())
        .bind(commit.as_str())
        .fetch_one(&admin)
        .await
        .expect("invalid build count");
        assert_eq!(build_count, 0);
        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox
             WHERE subject = 'hephaestus.build.requested.v1'
               AND payload->>'source_commit' = $1",
        )
        .bind(commit.as_str())
        .fetch_one(&admin)
        .await
        .expect("invalid build event count");
        assert_eq!(event_count, 0);
    }

    println!(
        "REAL_RECEIVE_MANUAL_UI_ORDERING=1 valid={valid} holder_pid={barrier_pid} receive_pid={receive_pid} manual_pid={manual_pid} received_builds={}",
        received.build_requests.len()
    );

    // UI captures and their build links are immutable evidence under migration
    // 0082. The disposable test-database runner owns final fixture disposal.
    drop(setup_service);
    receive_pool.close().await;
    manual_pool.close().await;
    admin.close().await;
}

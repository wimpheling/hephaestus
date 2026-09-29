use super::*;

#[tokio::test]
#[serial]
async fn app_role_receive_persists_reusable_ui_build_once() {
    let Some((pool, _admin_service, repository, temporary)) = fixture().await else {
        return;
    };
    let database_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("test URL");
    let owner = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'app-receive-owner')")
        .bind(owner.as_uuid())
        .execute(&pool)
        .await
        .expect("app receive user");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2)",
    )
    .bind(repository.project_id.as_uuid())
    .bind(owner.as_uuid())
    .execute(&pool)
    .await
    .expect("project maintainer");
    sqlx::query(
        "INSERT INTO repository_managers (repository_id, user_id)
         VALUES ($1, $2)",
    )
    .bind(repository.id.as_uuid())
    .bind(owner.as_uuid())
    .execute(&pool)
    .await
    .expect("repository manager");
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (commit, update) =
        commit_and_update_with_ui(&temporary, &repository, &config, STATIC_UI).await;
    let app_pool = app_role_pool(&database_url).await;
    let app_storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("application Git storage"),
    );
    let app_service = PgForgeRepository::new(app_pool, app_storage)
        .with_authorizer(Arc::new(PostgresMelangeAuthorizer));
    let identity = AuthenticatedIdentity::new(
        owner,
        "https://forge-app-role.example",
        "app-receive-owner",
        json!({"email_verified": true}),
        RequestId::new(),
    );
    let receive_id = ReceiveId::new();
    let first = app_service
        .accept_receive_as(
            &repository,
            receive_id,
            "app-role-receive",
            Some(&identity),
            std::slice::from_ref(&update),
        )
        .await
        .expect("application role receive");
    assert_eq!(first.build_requests.len(), 1);

    let build_id = first.build_requests[0].as_uuid();
    let parsed = agent_config::parse(config.as_bytes());
    let agent = parsed.config.expect("valid agent config");
    let base_hash = agent_config::build_identity::base_build_definition_hash(
        agent.build.as_ref().expect("build config"),
    )
    .expect("base build identity");
    let ui_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT normalized_ui_hash FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("captured UI hash");
    let ui_hash: [u8; 32] = ui_hash.try_into().expect("UI hash width");
    let expected_hash =
        agent_config::build_identity::ui_build_definition_hash(base_hash, ui_hash, None);
    let stored_hash: Vec<u8> =
        sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
            .bind(build_id)
            .fetch_one(&pool)
            .await
            .expect("stored derived build identity");
    assert_eq!(stored_hash, expected_hash);
    let link_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM build_request_ui_source_manifests
         WHERE build_request_id = $1 AND source_manifest_revision_id = (
             SELECT id FROM ui_source_manifest_revisions
             WHERE repository_id = $2 AND source_commit = $3
         )",
    )
    .bind(build_id)
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("exact UI build link count");
    assert_eq!(link_count, 1);
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = 'hephaestus.build.requested.v1' AND aggregate_id = $1",
    )
    .bind(build_id)
    .fetch_one(&pool)
    .await
    .expect("build outbox count");
    assert_eq!(event_count, 1);

    let second_receive = app_service
        .accept_receive_as(
            &repository,
            ReceiveId::new(),
            "app-role-receive",
            Some(&identity),
            std::slice::from_ref(&update),
        )
        .await
        .expect("application role same-build receive");
    assert_eq!(second_receive.build_requests, first.build_requests);
    let second_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = 'hephaestus.build.requested.v1' AND aggregate_id = $1",
    )
    .bind(build_id)
    .fetch_one(&pool)
    .await
    .expect("same-build outbox count");
    assert_eq!(second_event_count, 1);

    let replay = app_service
        .accept_receive_as(
            &repository,
            receive_id,
            "app-role-receive",
            Some(&identity),
            std::slice::from_ref(&update),
        )
        .await
        .expect("application role receive replay");
    assert_eq!(replay.build_requests, first.build_requests);
    let replay_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = 'hephaestus.build.requested.v1' AND aggregate_id = $1",
    )
    .bind(build_id)
    .fetch_one(&pool)
    .await
    .expect("replay build outbox count");
    assert_eq!(replay_event_count, 1);
    cleanup(&pool, repository).await;
}

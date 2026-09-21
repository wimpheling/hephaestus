//! Opt-in `PostgreSQL` and `JetStream` receive-processing coverage.

use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{
    CommitSha, GitRef, OrganizationId, ReceiveId, RefUpdate, Repository, RuntimeReceiveProvenance,
};
use forge_postgres::PgForgeRepository;
use forge_service::{
    CreateRepository, ForgeNatsOutboxPublisher, GitStorage, INSTANCE_RUN_REQUESTED_SUBJECT,
    ensure_forge_jetstream_topology,
};
use futures_util::StreamExt;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use run_domain::{CancelRun, Run, RunState, StartRun};
use run_orchestrator::{
    NatsCommandHandler, RunOrchestrator, RunRepository, VmSpecFactory, ensure_jetstream_topology,
};
use run_postgres::PgRunRepository;
use runtime_types::{CommandId, RunId};
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};
use tokio::process::Command;
use uuid::Uuid;
use vm_fake::FakeProvider;
use vm_trait::{GuestCommand, NetworkMode, RootFilesystem, VmError, VmId, VmResources, VmSpec};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_postgres::PostgresVolumeMetadataRepository;

const STATIC_UI: &str = r#"
version = 1

[[uis]]
key = "assistant"
scope = "project"
label = "Assistant"
icon = "chat"
presentation = "iframe"
route_base = "assistant"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"
"#;

#[tokio::test]
#[serial]
async fn accepted_ui_capture_links_build_and_replays_without_git() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (commit, update) =
        commit_and_update_with_ui(&temporary, &repository, &config, STATIC_UI).await;
    let receive_id = ReceiveId::new();
    let first = service
        .accept_receive(
            &repository,
            receive_id,
            "integration-user",
            std::slice::from_ref(&update),
        )
        .await
        .expect("accepted UI receive");
    assert_eq!(first.build_requests.len(), 1);

    let (revision_id, stored_receive, ui_hash): (Uuid, Uuid, Vec<u8>) = sqlx::query_as(
        "SELECT id, receive_id, normalized_ui_hash
         FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("stored valid UI revision");
    assert_eq!(stored_receive, receive_id.as_uuid());
    assert_eq!(ui_hash.len(), 32);

    let link: (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT build_request_id, source_manifest_revision_id, source_commit
         FROM build_request_ui_source_manifests
         WHERE build_request_id = $1",
    )
    .bind(first.build_requests[0].as_uuid())
    .fetch_one(&pool)
    .await
    .expect("build UI link");
    assert_eq!(link.0, first.build_requests[0].as_uuid());
    assert_eq!(link.1, revision_id);
    assert_eq!(link.2, commit.as_str());

    let parsed = agent_config::parse(config.as_bytes());
    let agent = parsed.config.expect("valid agent config");
    let base_hash = agent_config::build_identity::base_build_definition_hash(
        agent.build.as_ref().expect("build config"),
    )
    .expect("base build identity");
    let ui_hash: [u8; 32] = ui_hash.try_into().expect("UI hash width");
    let expected_hash =
        agent_config::build_identity::ui_build_definition_hash(base_hash, ui_hash, None);
    let stored_build_hash: Vec<u8> =
        sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
            .bind(first.build_requests[0].as_uuid())
            .fetch_one(&pool)
            .await
            .expect("UI build identity");
    assert_eq!(stored_build_hash, expected_hash.to_vec());

    tokio::fs::remove_dir_all(temporary.path().join("repositories"))
        .await
        .expect("remove Git storage for replay");
    let replay = service
        .accept_receive(
            &repository,
            receive_id,
            "integration-user",
            std::slice::from_ref(&update),
        )
        .await
        .expect("replay without Git storage");
    assert_eq!(replay.build_requests, first.build_requests);
    assert_eq!(replay.run_requests, first.run_requests);

    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
async fn runtime_receive_persists_provenance_suppresses_origin_and_replays() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let (origin_attachment, instance_id, project_id): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT id, instance_id, project_id
         FROM agent_attachments
         WHERE repository_id = $1
         ORDER BY id
         LIMIT 1",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("origin attachment");
    let sibling_attachment = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector, trigger_policy)
         VALUES ($1, $2, $3, $4, 'refs/heads/*', 'push')",
    )
    .bind(sibling_attachment)
    .bind(instance_id)
    .bind(project_id)
    .bind(repository.id.as_uuid())
    .execute(&pool)
    .await
    .expect("sibling attachment");

    let (revision_id, release_id, release_agent_id): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT revision.id, release.id, release_agent.id
         FROM agent_instance_revisions AS revision
         JOIN release_agents AS release_agent ON release_agent.id = revision.release_agent_id
         JOIN releases AS release ON release.id = release_agent.release_id
         WHERE revision.instance_id = $1",
    )
    .bind(instance_id)
    .fetch_one(&pool)
    .await
    .expect("origin revision");
    let run_id = Uuid::new_v4();
    let command_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, state, created_at, updated_at,
          instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, requires_state)
         VALUES ($1, $2, 'queued', now(), now(), $3, $4, $5, $6, $7,
                 'normal', false)",
    )
    .bind(run_id)
    .bind(command_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(origin_attachment)
    .execute(&pool)
    .await
    .expect("runtime run");
    let snapshot_id = Uuid::new_v4();
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
    .execute(&pool)
    .await
    .expect("runtime snapshot");
    let runtime_session_id = Uuid::new_v4();
    let runtime_credential_hash =
        [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    sqlx::query(
        "INSERT INTO runtime_authority_sessions
         (id, snapshot_id, run_id, instance_id, instance_revision_id,
          attachment_id, identity_hash, snapshot_hash, issuance_generation,
          credential_hash, status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9, 'active',
                 now(), now() + interval '10 minutes', now())",
    )
    .bind(runtime_session_id)
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(origin_attachment)
    .bind([2_u8; 32].as_slice())
    .bind([1_u8; 32].as_slice())
    .bind(runtime_credential_hash.as_slice())
    .execute(&pool)
    .await
    .expect("runtime session");

    let (_, update) = commit_and_update(
        &temporary,
        &repository,
        &valid_config(repository.id.as_uuid()),
    )
    .await;
    seed_runtime_receive_authority_rows(
        &pool,
        run_id,
        instance_id,
        revision_id,
        release_id,
        release_agent_id,
        origin_attachment,
        repository.id.as_uuid(),
        update.new_commit.as_ref().expect("runtime commit").as_str(),
        snapshot_id,
    )
    .await;
    let receive_id = ReceiveId::new();
    let first = service
        .accept_runtime_receive(
            &repository,
            receive_id,
            RuntimeReceiveProvenance { runtime_session_id },
            std::slice::from_ref(&update),
        )
        .await
        .expect("runtime receive");
    assert!(first.run_requests.is_empty());

    let stored: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT runtime_session_id, runtime_attachment_id
         FROM git_receives WHERE id = $1",
    )
    .bind(receive_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("runtime receive provenance");
    assert_eq!(stored, (Some(runtime_session_id), Some(origin_attachment)));

    let wrong_repository = forge_domain::RepositoryId::new();
    let wrong_repository_resolution: Option<(Option<Uuid>,)> =
        sqlx::query_as("SELECT * FROM resolve_runtime_receive_attachment($1, $2)")
            .bind(runtime_session_id)
            .bind(wrong_repository.as_uuid())
            .fetch_optional(&pool)
            .await
            .expect("wrong repository resolution");
    assert!(wrong_repository_resolution.is_none());

    let detached_session = seed_detached_runtime_session(
        &pool,
        &repository,
        instance_id,
        revision_id,
        release_id,
        release_agent_id,
    )
    .await;
    let detached = service
        .accept_runtime_receive(
            &repository,
            ReceiveId::new(),
            RuntimeReceiveProvenance {
                runtime_session_id: detached_session,
            },
            std::slice::from_ref(&update),
        )
        .await
        .expect("runtime receive without originating attachment");
    assert!(detached.run_requests.is_empty());
    let detached_provenance: Option<Uuid> = sqlx::query_scalar(
        "SELECT runtime_attachment_id FROM git_receives
         WHERE runtime_session_id = $1",
    )
    .bind(detached_session)
    .fetch_one(&pool)
    .await
    .expect("detached runtime provenance");
    assert_eq!(detached_provenance, None);

    let duplicate_session = service
        .accept_runtime_receive(
            &repository,
            receive_id,
            RuntimeReceiveProvenance {
                runtime_session_id: detached_session,
            },
            std::slice::from_ref(&update),
        )
        .await;
    assert!(matches!(
        duplicate_session,
        Err(forge_service::ForgeRepositoryError::ReceiveConflict(_))
    ));

    let replay = service
        .accept_runtime_receive(
            &repository,
            receive_id,
            RuntimeReceiveProvenance { runtime_session_id },
            std::slice::from_ref(&update),
        )
        .await
        .expect("runtime receive replay");
    assert_eq!(replay.run_requests, first.run_requests);

    cleanup(&pool, repository).await;
}

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

#[tokio::test]
#[serial]
async fn concurrent_receives_same_repository_serialize_ui_capture() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (commit, update) =
        commit_and_update_with_ui(&temporary, &repository, &config, STATIC_UI).await;
    let first_service = service.clone();
    let second_service = service.clone();
    let first_repository = repository.clone();
    let second_repository = repository.clone();
    let first_update = update.clone();
    let second_update = update.clone();
    let joined = tokio::time::timeout(Duration::from_secs(30), async move {
        tokio::join!(
            first_service.accept_receive(
                &first_repository,
                ReceiveId::new(),
                "concurrent-first",
                std::slice::from_ref(&first_update),
            ),
            second_service.accept_receive(
                &second_repository,
                ReceiveId::new(),
                "concurrent-second",
                std::slice::from_ref(&second_update),
            ),
        )
    })
    .await
    .expect("concurrent receives must not deadlock");
    let first = joined.0.expect("first concurrent receive");
    let second = joined.1.expect("second concurrent receive");
    assert_eq!(first.build_requests, second.build_requests);

    let captures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("capture count");
    assert_eq!(captures, 1, "same commit has one immutable capture");
    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
async fn invalid_ui_capture_accepts_ref_and_run_but_skips_build() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let invalid_ui = "version = 1\nunknown_field = \"invalid\"\n";
    let (commit, update) =
        commit_and_update_with_ui(&temporary, &repository, &config, invalid_ui).await;
    let receive = service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("invalid UI receive remains accepted");
    assert!(receive.build_requests.is_empty());
    assert_eq!(receive.run_requests.len(), 1);
    let (status, diagnostics): (String, serde_json::Value) = sqlx::query_as(
        "SELECT status, diagnostics FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("invalid UI revision");
    assert_eq!(status, "invalid");
    assert!(!diagnostics.as_array().expect("diagnostic array").is_empty());
    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM build_request_ui_source_manifests
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("invalid UI build links");
    assert_eq!(linked, 0);
    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
async fn invalid_ui_capture_is_retained_without_agent_config() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let invalid_ui = "version = 1\nunknown_field = \"invalid\"\n";
    let (commit, update) =
        commit_and_update_files(&temporary, &repository, None, Some(invalid_ui)).await;
    let receive = service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("invalid UI without agent remains accepted");
    assert!(receive.build_requests.is_empty());
    assert_eq!(receive.run_requests.len(), 1);
    let status: String = sqlx::query_scalar(
        "SELECT status FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("retained invalid UI revision");
    assert_eq!(status, "invalid");
    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
async fn same_commit_on_multiple_refs_reuses_capture_and_links_each_build() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid()).replace(
        "triggers = [\"refs/heads/main\"]",
        "triggers = [\"refs/heads/main\", \"refs/heads/feature\"]",
    );
    let (commit, main_update) =
        commit_and_update_with_ui(&temporary, &repository, &config, STATIC_UI).await;
    let feature_update = RefUpdate {
        git_ref: GitRef::parse("refs/heads/feature").expect("feature ref"),
        old_commit: None,
        new_commit: Some(commit.clone()),
    };
    let receive = service
        .accept_receive(
            &repository,
            ReceiveId::new(),
            "integration-user",
            &[main_update, feature_update],
        )
        .await
        .expect("accepted multi-ref receive");
    assert_eq!(receive.build_requests.len(), 2);
    let revisions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("single shared UI revision");
    assert_eq!(revisions, 1);
    let links: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM build_request_ui_source_manifests
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("two UI build links");
    assert_eq!(links, 2);
    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
async fn persists_exact_config_and_deduplicates_receive() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (commit, update) = commit_and_update(&temporary, &repository, &config).await;
    let receive_id = ReceiveId::new();
    let first = service
        .accept_receive(
            &repository,
            receive_id,
            "integration-user",
            std::slice::from_ref(&update),
        )
        .await
        .expect("accepted receive");
    let duplicate = service
        .accept_receive(&repository, receive_id, "integration-user", &[update])
        .await
        .expect("duplicate receive");

    assert_eq!(first.run_requests.len(), 1);
    assert_eq!(duplicate.run_requests, first.run_requests);
    let stored_commit: String = sqlx::query_scalar(
        "SELECT commit_sha FROM agent_config_revisions WHERE repository_id = $1",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored config commit");
    let starts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_type = 'forge' AND subject = $1
           AND aggregate_id = $2",
    )
    .bind(INSTANCE_RUN_REQUESTED_SUBJECT)
    .bind(first.run_requests[0].id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("start command count");
    assert_eq!(stored_commit, commit.as_str());
    assert_eq!(starts, 1);
    let reusable_requests: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests
         WHERE receive_id = $1 AND request_kind = 'instance_normal'",
    )
    .bind(receive_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("exact reusable request count");
    assert_eq!(reusable_requests, 1);
    let reusable_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = 'hephaestus.instance.run.requested.v1'
           AND payload->>'receive_id' = $1",
    )
    .bind(receive_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("reusable run event count");
    assert_eq!(reusable_events, 1);

    let work = temporary.path().join("work");
    let invalid = config.replace("version = 2", "version = 99");
    tokio::fs::write(work.join("agent.toml"), invalid)
        .await
        .expect("invalid agent configuration");
    git(&work, &["add", "agent.toml"]).await;
    git(&work, &["commit", "-m", "unsupported config"]).await;
    let invalid_commit =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("invalid commit");
    let bare = temporary
        .path()
        .join("repositories")
        .join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    let invalid_result = service
        .accept_receive(
            &repository,
            ReceiveId::new(),
            "integration-user",
            &[RefUpdate {
                git_ref: GitRef::parse("refs/heads/main").expect("updated ref"),
                old_commit: Some(commit),
                new_commit: Some(invalid_commit),
            }],
        )
        .await
        .expect("invalid configuration receive");
    assert_eq!(invalid_result.invalid_configurations, 1);
    assert_eq!(
        invalid_result.run_requests.len(),
        1,
        "target agent.toml validity must not control attached instances"
    );
    let diagnostic_code: String = sqlx::query_scalar(
        "SELECT diagnostics->0->>'code'
         FROM agent_config_revisions
         WHERE repository_id = $1 AND status = 'invalid'",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored diagnostic");
    assert_eq!(diagnostic_code, "unsupported_version");

    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
async fn forge_outbox_retry_is_deduplicated_by_jetstream() {
    let (Ok(nats_url), Some((pool, service, repository, temporary))) =
        (std::env::var("HEPHAESTUS_NATS_TEST_URL"), fixture().await)
    else {
        return;
    };
    // Opt-in test binaries may share one explicitly configured database.
    sqlx::query(
        "UPDATE outbox SET published_at = now()
         WHERE aggregate_type = 'forge' AND published_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("isolate forge outbox fixture");
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (_, update) = commit_and_update(&temporary, &repository, &config).await;
    service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("accepted receive");
    let command_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM outbox
         WHERE published_at IS NULL
           AND subject IN (
               'hephaestus.build.requested.v1',
               'hephaestus.instance.run.requested.v1',
               'hephaestus.run.start'
           )
         ORDER BY occurred_at, id",
    )
    .fetch_all(&pool)
    .await
    .expect("exact receive command identities");
    assert!(!command_ids.is_empty());

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let stream_name = format!("HEPH_PHASE2_TEST_{}", repository.id.as_uuid().simple());
    let mut stream = context
        .create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![String::from("hephaestus.>")],
            duplicate_window: Duration::from_secs(60),
            ..Default::default()
        })
        .await
        .expect("isolated forge stream");
    let publisher = ForgeNatsOutboxPublisher::new(context.clone());
    assert_eq!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("first publication"),
        command_ids.len()
    );
    assert_eq!(
        stream.info().await.expect("stream state").state.messages,
        u64::try_from(command_ids.len()).expect("bounded command count")
    );
    sqlx::query("UPDATE outbox SET published_at = NULL WHERE id = ANY($1)")
        .bind(&command_ids)
        .execute(&pool)
        .await
        .expect("simulate acknowledgement loss");
    assert_eq!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("retry publication"),
        command_ids.len()
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("deduplicated state")
            .state
            .messages,
        u64::try_from(command_ids.len()).expect("bounded command count")
    );

    context
        .delete_stream(&stream_name)
        .await
        .expect("delete isolated stream");
    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
// This deliberately crosses every durable boundary in one scenario.
#[allow(clippy::too_many_lines)]
async fn pushed_config_publishes_command_and_starts_vm() {
    let (Ok(nats_url), Some((pool, service, repository, temporary))) =
        (std::env::var("HEPHAESTUS_NATS_TEST_URL"), fixture().await)
    else {
        return;
    };
    // The publisher scans the shared forge outbox, so pre-existing fixtures
    // must not supply an older StartRun delivery to this scenario's consumer.
    sqlx::query(
        "UPDATE outbox SET published_at = now()
         WHERE aggregate_type = 'forge' AND published_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("isolate forge start-run fixture");
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (_, update) = commit_and_update(&temporary, &repository, &config).await;
    let receive = service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("accepted receive");
    let request = receive.run_requests[0].clone();

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let consumer = ensure_jetstream_topology(&context)
        .await
        .expect("run topology");
    ensure_forge_jetstream_topology(&context)
        .await
        .expect("forge topology");

    let run_repository = Arc::new(PgRunRepository::new(pool.clone()));
    let volume_root = temporary.path().join("volumes");
    let volumes = Arc::new(
        LocalVolumeStore::new(
            Arc::new(PostgresVolumeMetadataRepository::new(pool.clone())),
            LocalVolumeConfig {
                volume_root,
                transient_runtime_roots: Vec::new(),
                host_id: String::from("phase2-integration"),
                lease_duration: Duration::from_secs(30),
                mkfs_ext4: std::path::PathBuf::from("/usr/bin/mkfs.ext4"),
            },
        )
        .expect("volume configuration"),
    );
    volumes.initialize().await.expect("volume store");
    let guest_root = temporary.path().join("guest-root");
    tokio::fs::create_dir(&guest_root)
        .await
        .expect("guest root");
    let orchestrator = Arc::new(RunOrchestrator::new(
        run_repository.clone(),
        volumes,
        Arc::new(FakeProvider::new()),
        Arc::new(TestSpecFactory { root: guest_root }),
        16 * 1024 * 1024,
    ));
    let handler = NatsCommandHandler::new(Arc::clone(&orchestrator));
    let publisher = ForgeNatsOutboxPublisher::new(context.clone());
    assert!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("publish receive commands")
            > 0
    );

    let mut messages = consumer.messages().await.expect("command messages");
    let message = messages
        .next()
        .await
        .expect("start delivery")
        .expect("valid start delivery");
    let handler_task = tokio::spawn(async move { handler.handle(&message).await });
    wait_for_run_state(&pool, request.command.run_id, "running").await;
    orchestrator
        .cancel_run(&CancelRun {
            command_id: CommandId::new(),
            run_id: request.command.run_id,
            reason: String::from("complete integration test"),
        })
        .await
        .expect("stop started VM");
    handler_task
        .await
        .expect("join command handler")
        .expect("handle start command");
    assert_eq!(
        run_repository
            .get(request.command.run_id)
            .await
            .expect("completed run")
            .state,
        RunState::CleanedUp
    );
    let reached_running: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM run_events
           WHERE run_id = $1 AND event_type = 'run.running'
         )",
    )
    .bind(request.command.run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("running transition");
    assert!(reached_running, "published command did not start the VM");

    cleanup_run(&pool, &request.command).await;
    cleanup(&pool, repository).await;
    context
        .delete_stream("HEPH_RUN_COMMANDS")
        .await
        .expect("delete command stream");
    context
        .delete_stream("HEPHAESTUS_GIT_EVENTS")
        .await
        .expect("delete Git event stream");
}

struct TestSpecFactory {
    root: std::path::PathBuf,
}

#[async_trait::async_trait]
impl VmSpecFactory for TestSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        Ok(VmSpec {
            id: VmId(run.id.to_string()),
            root: RootFilesystem::Directory {
                host_path: self.root.clone(),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 128,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/bin/true"),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: None,
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: None,
            labels: BTreeMap::new(),
        })
    }
}

async fn wait_for_run_state(pool: &PgPool, run_id: RunId, expected: &str) {
    for _ in 0..200 {
        let state = sqlx::query_scalar::<_, String>("SELECT state FROM runs WHERE id = $1")
            .bind(run_id.as_uuid())
            .fetch_optional(pool)
            .await
            .expect("load run state");
        if state.as_deref() == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run_id} did not reach {expected}");
}

async fn cleanup_run(pool: &PgPool, command: &StartRun) {
    let volume_id: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT volume_id FROM runs WHERE id = $1")
            .bind(command.run_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("run volume");
    sqlx::query("DELETE FROM outbox WHERE aggregate_type = 'run' AND aggregate_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run outbox");
    sqlx::query("DELETE FROM run_events WHERE run_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run events");
    sqlx::query("DELETE FROM command_inbox WHERE payload->>'run_id' = $1")
        .bind(command.run_id.to_string())
        .execute(pool)
        .await
        .expect("delete command inbox");
    if let Some(volume_id) = volume_id {
        sqlx::query("DELETE FROM agent_instance_volume_leases WHERE volume_id = $1")
            .bind(volume_id)
            .execute(pool)
            .await
            .expect("delete volume leases");
    }
    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run");
}

async fn fixture() -> Option<(PgPool, PgForgeRepository, Repository, tempfile::TempDir)> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("PostgreSQL integration connection");
    let temporary = tempfile::tempdir().expect("temporary directory");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("Git storage"),
    );
    let service = PgForgeRepository::new(pool.clone(), storage);
    service.initialize().await.expect("forge migrations");
    let organization_id = OrganizationId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind("forge-service-integration")
        .execute(&pool)
        .await
        .expect("organization");
    let project = service
        .create_project_trusted(organization_id, "forge-service-integration")
        .await
        .expect("project");
    let repository = service
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("repository");
    seed_catalog_images(&pool, repository.id.as_uuid()).await;
    Some((pool, service, repository, temporary))
}

async fn app_role_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect application role")
}

async fn seed_catalog_images(pool: &PgPool, repository_id: Uuid) {
    for (key, display_name, image_reference) in [
        (
            fixture_image_key("build", repository_id),
            String::from("Forge test build image"),
            fixture_image_reference("forge-build"),
        ),
        (
            fixture_image_key("runtime", repository_id),
            String::from("Forge test runtime image"),
            fixture_image_reference("forge-runtime"),
        ),
    ] {
        sqlx::query(
            "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version)
             VALUES ($1, $2, $3, $4, '[]'::jsonb, ARRAY['x86_64'],
                     'available', '{}'::jsonb, 'test/v1')",
        )
        .bind(Uuid::new_v4())
        .bind(key)
        .bind(display_name)
        .bind(image_reference)
        .execute(pool)
        .await
        .expect("seed fixture OCI image");
    }
}

async fn seed_reusable_attachment(pool: &PgPool, repository: &Repository) {
    let build_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    seed_reusable_release(
        pool,
        repository,
        build_id,
        family_id,
        release_id,
        release_agent_id,
    )
    .await;
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    seed_attached_instance(
        pool,
        repository,
        family_id,
        release_agent_id,
        instance_id,
        revision_id,
        attachment_id,
    )
    .await;
}

async fn seed_reusable_release(
    pool: &PgPool,
    repository: &Repository,
    build_id: Uuid,
    family_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(repository.id.as_uuid())
    .bind("d".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed reusable build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'attached')",
    )
    .bind(family_id)
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("seed family");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                 '{}', $6, $7, 'published', now())",
    )
    .bind(release_id)
    .bind(repository.id.as_uuid())
    .bind("d".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'attached', 'Attached', '{}', $4,
                 '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release agent");
}

// Explicit fixture IDs keep every seeded foreign-key edge visible in the test.
#[allow(clippy::too_many_arguments)]
async fn seed_attached_instance(
    pool: &PgPool,
    repository: &Repository,
    family_id: Uuid,
    release_agent_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(instance_id)
    .bind(repository.project_id.as_uuid())
    .bind(family_id)
    .bind(format!("attached-{}", instance_id.simple()))
    .execute(pool)
    .await
    .expect("seed instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5,
                 'platform/v1', true)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push')",
    )
    .bind(attachment_id)
    .bind(instance_id)
    .bind(repository.project_id.as_uuid())
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("seed attachment");
}

// This fixture uses a superuser-only replica-mode insert for the immutable
// typed Git snapshot because forge-postgres does not own release capability
// publication. Production rows are created by the runtime authority adapter;
// the receive test only needs the persisted join graph to exercise its
// security-definer lookup against a real PostgreSQL body.
#[allow(clippy::too_many_arguments)]
async fn seed_runtime_receive_authority_rows(
    pool: &PgPool,
    run_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Uuid,
    repository_id: Uuid,
    commit: &str,
    snapshot_id: Uuid,
) {
    let mut transaction = pool.begin().await.expect("begin authority fixture");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *transaction)
        .await
        .expect("enable fixture replica mode");
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
         (snapshot_id, instance_revision_id, binding_id, repository_id,
          grammar_version, git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, expected_parent, normalized_hash)
         VALUES ($1, $2, $3, $4, 1, ARRAY['receive'],
                 ARRAY['refs/heads/*'], ARRAY['**'], 'fast_forward_only',
                 false, false, false, false, false, false, false, false,
                 1, 1, 1, 1, false, NULL, $5)",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(Uuid::new_v4())
    .bind(repository_id)
    .bind([9_u8; 32].as_slice())
    .execute(&mut *transaction)
    .await
    .expect("typed Git authority snapshot");
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
    .expect("run provenance");
    transaction
        .commit()
        .await
        .expect("commit authority fixture");
}

async fn seed_detached_runtime_session(
    pool: &PgPool,
    repository: &Repository,
    instance_id: Uuid,
    revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
) -> Uuid {
    let run_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let now = time::OffsetDateTime::now_utc();
    let expires_at = now + time::Duration::minutes(10);
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, state, created_at, updated_at,
          instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, requires_state)
         VALUES ($1, $2, 'queued', $3, $3, $4, $5, $6, $7, NULL, 'update', false)",
    )
    .bind(run_id)
    .bind(Uuid::new_v4())
    .bind(now)
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .execute(pool)
    .await
    .expect("detached runtime run");
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
    .bind([14_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("detached runtime snapshot");
    let credential_hash = [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    sqlx::query(
        "INSERT INTO runtime_authority_sessions
         (id, snapshot_id, run_id, instance_id, instance_revision_id,
          attachment_id, identity_hash, snapshot_hash, issuance_generation,
          credential_hash, status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, NULL, $6, $7, 1, $8, 'active', $9, $10, $9)",
    )
    .bind(session_id)
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind([15_u8; 32].as_slice())
    .bind([14_u8; 32].as_slice())
    .bind(credential_hash.as_slice())
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("detached runtime session");
    let mut transaction = pool.begin().await.expect("begin detached runtime graph");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *transaction)
        .await
        .expect("enable detached fixture mode");
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
         (snapshot_id, instance_revision_id, binding_id, repository_id,
          grammar_version, git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, expected_parent, normalized_hash)
         VALUES ($1, $2, $3, $4, 1, ARRAY['receive'],
                 ARRAY['refs/heads/main'], ARRAY['**'], 'fast_forward_only',
                 false, false, false, false, false, false, false, false,
                 1, 1, 1, 1, false, NULL, $5)",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(Uuid::new_v4())
    .bind(repository.id.as_uuid())
    .bind([16_u8; 32].as_slice())
    .execute(&mut *transaction)
    .await
    .expect("detached runtime Git snapshot");
    transaction
        .commit()
        .await
        .expect("commit detached runtime graph");
    session_id
}

async fn commit_and_update(
    temporary: &tempfile::TempDir,
    repository: &Repository,
    config: &str,
) -> (CommitSha, RefUpdate) {
    commit_and_update_files(temporary, repository, Some(config), None).await
}

async fn commit_and_update_with_ui(
    temporary: &tempfile::TempDir,
    repository: &Repository,
    config: &str,
    ui_manifest: &str,
) -> (CommitSha, RefUpdate) {
    commit_and_update_files(temporary, repository, Some(config), Some(ui_manifest)).await
}

async fn commit_and_update_files(
    temporary: &tempfile::TempDir,
    repository: &Repository,
    config: Option<&str>,
    ui_manifest: Option<&str>,
) -> (CommitSha, RefUpdate) {
    let work = temporary.path().join("work");
    tokio::fs::create_dir(&work).await.expect("work directory");
    git(&work, &["init", "--initial-branch=main"]).await;
    git(&work, &["config", "user.name", "Hephaestus Test"]).await;
    git(
        &work,
        &["config", "user.email", "hephaestus@example.invalid"],
    )
    .await;
    if let Some(config) = config {
        tokio::fs::write(work.join("agent.toml"), config)
            .await
            .expect("agent configuration");
    }
    if let Some(ui_manifest) = ui_manifest {
        tokio::fs::write(work.join("heph.ui.toml"), ui_manifest)
            .await
            .expect("UI manifest");
    }
    git(&work, &["add", "."]).await;
    git(&work, &["commit", "-m", "agent config"]).await;
    let commit =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("commit ID");
    let bare = temporary
        .path()
        .join("repositories")
        .join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    (
        commit.clone(),
        RefUpdate {
            git_ref: GitRef::parse("refs/heads/main").expect("updated ref"),
            old_commit: None,
            new_commit: Some(commit),
        },
    )
}

async fn cleanup(pool: &PgPool, repository: Repository) {
    let retains_instance_history: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM agent_attachments WHERE repository_id = $1
         )",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("instance provenance retention check");
    if retains_instance_history {
        // Reusable attachment/release provenance is deliberately permanent;
        // this random fixture cannot be deleted without violating that model.
        return;
    }
    sqlx::query(
        "DELETE FROM outbox WHERE aggregate_type = 'forge'
         AND (
           aggregate_id IN (SELECT id FROM run_requests WHERE repository_id = $1)
           OR aggregate_id IN (SELECT id FROM agent_config_revisions WHERE repository_id = $1)
           OR aggregate_id IN (SELECT id FROM git_receives WHERE repository_id = $1)
         )",
    )
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("delete forge outbox");
    sqlx::query("DELETE FROM run_requests WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run requests");
    sqlx::query("DELETE FROM agent_config_revisions WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete config revisions");
    sqlx::query("DELETE FROM git_refs WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete current refs");
    sqlx::query(
        "DELETE FROM git_ref_updates
         WHERE receive_id IN (SELECT id FROM git_receives WHERE repository_id = $1)",
    )
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("delete ref updates");
    sqlx::query("DELETE FROM git_receives WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete receives");
    sqlx::query("DELETE FROM repositories WHERE id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete repository");
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(repository.project_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete project");
}

async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
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
        .expect("run Git");
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

fn valid_config(repository_id: Uuid) -> String {
    r#"
version = 2
[agent]
name = "Reviewer"
key = "reviewer"
[build]
image = { key = "__BUILD_IMAGE_KEY__" }
command = "/bin/build"
working_directory = "/source"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 512
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/reviewer"
kind = "executable"
[guest]
image = { key = "__RUNTIME_IMAGE_KEY__" }
command = "bin/reviewer"
arguments = []
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 256
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = true
[network]
profile = "disabled"
[triggers]
push = false
"#
    .replace(
        "__BUILD_IMAGE_KEY__",
        &fixture_image_key("build", repository_id),
    )
    .replace(
        "__RUNTIME_IMAGE_KEY__",
        &fixture_image_key("runtime", repository_id),
    )
}

fn fixture_image_key(kind: &str, repository_id: Uuid) -> String {
    format!("forge-{kind}-{}", repository_id.simple())
}

fn fixture_image_reference(kind: &str) -> String {
    let first = Uuid::new_v4().simple().to_string();
    let second = Uuid::new_v4().simple().to_string();
    format!("{kind}@sha256:{first}{second}")
}

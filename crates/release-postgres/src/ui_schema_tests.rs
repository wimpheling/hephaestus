//! Opt-in `PostgreSQL` coverage for the release-owned UI publication schema.
//!
//! This module covers the release-owned UI publication schema introduced by
//! migration 0084. It uses only bound SQL and the existing disposable
//! `PostgreSQL` fixture.

use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use tokio::{sync::oneshot, time::timeout};
use uuid::Uuid;

const COMMIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMMIT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Clone, Copy)]
struct ReleaseFixture {
    owner: Uuid,
    outsider: Uuid,
    release: Uuid,
    release_without_ui: Uuid,
    build: Uuid,
    build_without_ui: Uuid,
    source_revision: Uuid,
    source_revision_without_ui: Uuid,
    release_agent: Uuid,
    foreign_agent: Uuid,
    artifact: Uuid,
    foreign_artifact: Uuid,
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn release_ui_schema_enforces_identity_shape_lifecycle_and_rls() {
    let Some(url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("SKIPPED release UI schema: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let admin = admin_pool(&url).await;
    sqlx::migrate!("../../migrations")
        .run(&admin)
        .await
        .expect("apply migrations through 0084");
    let version: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&admin)
            .await
            .expect("read migration version");
    assert!(
        version >= 84,
        "release UI publication schema and catalog grants must be installed"
    );

    let fixture = seed_fixture(&admin).await;
    seed_valid_ui_rows(&admin, &fixture).await;
    assert_valid_rows(&admin, &fixture).await;
    assert_shape_constraints(&admin, &fixture).await;
    assert_immutable_rows(&admin, &fixture).await;
    assert_rls_isolation(&url, &fixture).await;

    sqlx::query(
        "UPDATE releases
         SET state = 'published', published_at = now(), publication_actor_id = $2
         WHERE id = $1 AND state = 'draft'",
    )
    .bind(fixture.release)
    .bind(fixture.owner)
    .execute(&admin)
    .await
    .expect("draft release publishes after UI rows are inserted");
    sqlx::query(
        "UPDATE releases
         SET state = 'published', published_at = now(), publication_actor_id = $2
         WHERE id = $1 AND state = 'draft'",
    )
    .bind(fixture.release_without_ui)
    .bind(fixture.owner)
    .execute(&admin)
    .await
    .expect("empty draft release publishes for source-snapshot guard");
    assert_state(&admin, fixture.release, "published").await;
    assert_insert_guard(&admin, &fixture, "published").await;

    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = now()
         WHERE id = $1 AND state = 'published'",
    )
    .bind(fixture.release)
    .execute(&admin)
    .await
    .expect("published release revokes");
    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = now()
         WHERE id = $1 AND state = 'published'",
    )
    .bind(fixture.release_without_ui)
    .execute(&admin)
    .await
    .expect("empty published release revokes");
    assert_state(&admin, fixture.release, "revoked").await;
    assert_insert_guard(&admin, &fixture, "revoked").await;
}

#[tokio::test]
#[serial]
async fn release_ui_draft_guard_serializes_publish_and_ui_insert() {
    let Some(url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("SKIPPED release UI draft guard: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let admin = admin_pool(&url).await;
    sqlx::migrate!("../../migrations")
        .run(&admin)
        .await
        .expect("apply migrations through 0084");
    let fixture = seed_fixture(&admin).await;
    seed_valid_ui_rows(&admin, &fixture).await;

    let mut insert_first = admin.begin().await.expect("begin insert-first transaction");
    let insert_first_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *insert_first)
        .await
        .expect("read insert-first PID");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(fixture.release_without_ui)
    .bind(fixture.build_without_ui)
    .bind(fixture.source_revision_without_ui)
    .execute(&mut *insert_first)
    .await
    .expect("insert source snapshot while draft lock is held");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'race-insert', 'global', 'Race insert', 'app', 'iframe',
                 'race-insert', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(fixture.release_without_ui)
    .execute(&mut *insert_first)
    .await
    .expect("insert UI descriptor while draft lock is held");

    let (publish_pid_tx, publish_pid_rx) = oneshot::channel();
    let publish_pool = admin.clone();
    let publish_release = fixture.release_without_ui;
    let publish_owner = fixture.owner;
    let publish_task = tokio::spawn(async move {
        let mut transaction = publish_pool.begin().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *transaction)
            .await?;
        publish_pid_tx.send(pid).expect("publish PID receiver");
        sqlx::query(
            "UPDATE releases
             SET state = 'published', published_at = now(), publication_actor_id = $2
             WHERE id = $1 AND state = 'draft'",
        )
        .bind(publish_release)
        .bind(publish_owner)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await
    });
    let publish_pid = publish_pid_rx.await.expect("publish PID");
    wait_until_blocked(&admin, publish_pid, insert_first_pid).await;
    insert_first
        .commit()
        .await
        .expect("commit insert before publish");
    publish_task
        .await
        .expect("publish task join")
        .expect("publish after insert commit");
    assert_state(&admin, fixture.release_without_ui, "published").await;
    let inserted_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1")
            .bind(fixture.release_without_ui)
            .fetch_one(&admin)
            .await
            .expect("count inserted descriptor");
    assert_eq!(inserted_count, 1);

    let mut publish_first = admin
        .begin()
        .await
        .expect("begin publish-first transaction");
    sqlx::query(
        "UPDATE releases
         SET state = 'published', published_at = now(), publication_actor_id = $2
         WHERE id = $1 AND state = 'draft'",
    )
    .bind(fixture.release)
    .bind(fixture.owner)
    .execute(&mut *publish_first)
    .await
    .expect("hold published release row lock");
    let publish_first_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *publish_first)
        .await
        .expect("read publish-first PID");
    let (insert_pid_tx, insert_pid_rx) = oneshot::channel();
    let insert_pool = admin.clone();
    let insert_release = fixture.release;
    let insert_artifact = fixture.artifact;
    let insert_task = tokio::spawn(async move {
        let mut transaction = insert_pool.begin().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *transaction)
            .await?;
        insert_pid_tx.send(pid).expect("insert PID receiver");
        sqlx::query(
            "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'docs', 'after-publish.html', $2, 'file', 'text/html')",
        )
        .bind(insert_release)
        .bind(insert_artifact)
        .execute(&mut *transaction)
        .await
        .map(|_| ())
    });
    let insert_pid = insert_pid_rx.await.expect("insert PID");
    wait_until_blocked(&admin, insert_pid, publish_first_pid).await;
    publish_first
        .commit()
        .await
        .expect("commit publish before blocked insert resumes");
    let insert_result = insert_task.await.expect("insert task join");
    assert_sqlstate(insert_result, "23000");
    let static_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_ui_static_files WHERE release_id = $1")
            .bind(fixture.release)
            .fetch_one(&admin)
            .await
            .expect("count static rows after rejected insert");
    assert_eq!(static_count, 1);
}

async fn wait_until_blocked(pool: &PgPool, pid: i32, expected_blocker: i32) {
    timeout(Duration::from_secs(5), async {
        loop {
            let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_blocking_pids($1)")
                .bind(pid)
                .fetch_one(pool)
                .await
                .expect("read blocking PostgreSQL PIDs");
            if blockers.contains(&expected_blocker) {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("transaction reached the expected lock barrier");
}

async fn admin_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
        .expect("connect PostgreSQL test database")
}

// Keep the disposable SQL fixture setup together for readable schema coverage.
#[allow(clippy::too_many_lines)]
async fn seed_fixture(pool: &PgPool) -> ReleaseFixture {
    let owner = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let private_repository = Uuid::new_v4();
    for (id, name) in [(owner, "ui-schema-owner"), (outsider, "ui-schema-outsider")] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .expect("seed user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'ui-schema-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(organization)
    .bind(owner)
    .execute(pool)
    .await
    .expect("seed organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'ui-schema')")
        .bind(project)
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project)
        .bind(owner)
        .execute(pool)
        .await
        .expect("seed project maintainer");
    for (id, name) in [
        (repository, "ui-schema-repository"),
        (private_repository, "ui-schema-private"),
    ] {
        sqlx::query(
            "INSERT INTO repositories
             (id, project_id, name, default_branch, is_public)
             VALUES ($1, $2, $3, 'refs/heads/main', false)",
        )
        .bind(id)
        .bind(project)
        .bind(name)
        .execute(pool)
        .await
        .expect("seed repository");
    }

    let release = Uuid::new_v4();
    let release_without_ui = Uuid::new_v4();
    let build = Uuid::new_v4();
    let build_without_ui = Uuid::new_v4();
    let source_revision = Uuid::new_v4();
    let source_revision_without_ui = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    let foreign_agent = Uuid::new_v4();
    let artifact = Uuid::new_v4();
    let foreign_artifact = Uuid::new_v4();

    seed_release_inputs(
        pool,
        owner,
        repository,
        release,
        build,
        source_revision,
        COMMIT_A,
        release_agent,
        artifact,
        true,
    )
    .await;
    seed_release_inputs(
        pool,
        owner,
        repository,
        release_without_ui,
        build_without_ui,
        source_revision_without_ui,
        COMMIT_B,
        foreign_agent,
        foreign_artifact,
        false,
    )
    .await;
    ReleaseFixture {
        owner,
        outsider,
        release,
        release_without_ui,
        build,
        build_without_ui,
        source_revision,
        source_revision_without_ui,
        release_agent,
        foreign_agent,
        artifact,
        foreign_artifact,
    }
}

#[allow(clippy::too_many_arguments)]
// Keep the release identity fixture together so its FK relationships are clear.
#[allow(clippy::too_many_lines)]
async fn seed_release_inputs(
    pool: &PgPool,
    owner: Uuid,
    repository: Uuid,
    release: Uuid,
    build: Uuid,
    source_revision: Uuid,
    commit: &str,
    release_agent: Uuid,
    artifact: Uuid,
    include_snapshot: bool,
) {
    let receive = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'ui-schema-test', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository)
    .bind(owner)
    .execute(pool)
    .await
    .expect("seed receive");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'succeeded', $6)",
    )
    .bind(build)
    .bind(repository)
    .bind(commit)
    .bind(receive)
    .bind([1_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("seed build request");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', $7, $8)",
    )
    .bind(source_revision)
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .bind("cccccccccccccccccccccccccccccccccccccccc")
    .bind([2_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed valid UI source capture");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit, source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(build)
    .bind(repository)
    .bind(commit)
    .bind(source_revision)
    .execute(pool)
    .await
    .expect("link build to UI source capture");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, $7, $8, $9, 'draft')",
    )
    .bind(release)
    .bind(repository)
    .bind(format!("v1.0.{}", &release.to_string()[..8]))
    .bind(commit)
    .bind(build)
    .bind([1_u8; 32].as_slice())
    .bind(json!({"version": 1}))
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed draft release");
    let family = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family)
    .bind(repository)
    .bind(format!("ui-agent-{}", &release.to_string()[..8]))
    .execute(pool)
    .await
    .expect("seed agent family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, $4, 'UI schema agent', '{}', $5, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(family)
    .bind(format!("ui-agent-{}", &release.to_string()[..8]))
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release agent");
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes,
          media_type, storage_key)
         VALUES ($1, $2, 'dist/index.html', 'file', 420, $3, 12,
                 'text/html', $4)",
    )
    .bind(artifact)
    .bind(release)
    .bind([7_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed release artifact");
    if include_snapshot {
        sqlx::query(
            "INSERT INTO release_ui_source_snapshots
             (release_id, build_request_id, source_manifest_revision_id)
             VALUES ($1, $2, $3)",
        )
        .bind(release)
        .bind(build)
        .bind(source_revision)
        .execute(pool)
        .await
        .expect("seed release UI source snapshot");
    }
}

async fn seed_valid_ui_rows(pool: &PgPool, fixture: &ReleaseFixture) {
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'docs', 'global', 'Docs', 'book', 'iframe',
                 'docs', 'index.html', 1, 'no_store', 'static'),
                ($1, 'service', 'global', 'Service', 'app', 'full_page',
                 'service', 'index.html', 1, 'no_store', 'managed_service')",
    )
    .bind(fixture.release)
    .execute(pool)
    .await
    .expect("seed UI descriptors");
    sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(fixture.release)
    .bind(fixture.artifact)
    .execute(pool)
    .await
    .expect("seed static UI file");
    sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'service', 'ui-service', '/service', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(pool)
    .await
    .expect("seed managed UI service");
    sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
         VALUES ($1, 'service', 'health', 'ui-service', 'GET', '/service/health', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(pool)
    .await
    .expect("seed UI API binding");
}

async fn assert_valid_rows(pool: &PgPool, fixture: &ReleaseFixture) {
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_static_files WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1)",
    )
    .bind(fixture.release)
    .fetch_one(pool)
    .await
    .expect("read valid UI rows");
    assert_eq!(counts, (1, 2, 1, 1, 1));
}

// Keep the negative SQL assertions together to show the complete constraint set.
#[allow(clippy::too_many_lines)]
async fn assert_shape_constraints(pool: &PgPool, fixture: &ReleaseFixture) {
    let cross_artifact = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'cross-artifact', $2, 'file', 'text/html')",
    )
    .bind(fixture.release)
    .bind(fixture.foreign_artifact)
    .execute(pool)
    .await;
    assert_sqlstate(cross_artifact, "23503");

    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'service-cross-agent', 'global', 'Cross agent', 'app',
                 'full_page', 'service-cross-agent', 'index.html', 1,
                 'no_store', 'managed_service')",
    )
    .bind(fixture.release)
    .execute(pool)
    .await
    .expect("seed cross-agent managed descriptor");
    let cross_agent = sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'service-cross-agent', 'ui-service', '/cross-agent', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.foreign_agent)
    .execute(pool)
    .await;
    assert_sqlstate(cross_agent, "23503");

    let cross_build = sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(fixture.release_without_ui)
    .bind(fixture.build)
    .bind(fixture.source_revision)
    .execute(pool)
    .await;
    assert_sqlstate(cross_build, "23503");

    let no_snapshot = sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'missing-source', 'global', 'Missing', 'app', 'iframe',
                 'missing', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(fixture.release_without_ui)
    .execute(pool)
    .await;
    assert_sqlstate(no_snapshot, "23503");

    for (key, route_base) in [
        ("double-slash", "bad//path"),
        ("dot-segment", "bad/../path"),
    ] {
        let result = sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, $2, 'global', 'Bad', 'app', 'iframe', $3,
                     'index.html', 1, 'no_store', 'static')",
        )
        .bind(fixture.release)
        .bind(key)
        .bind(route_base)
        .execute(pool)
        .await;
        assert_sqlstate(result, "23514");
    }
    for (route, key) in [("//service", "double-route"), ("/../service", "dot-route")] {
        let result = sqlx::query(
            "INSERT INTO release_ui_api_bindings
             (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
             VALUES ($1, 'service', $2, 'ui-service', 'GET', $3, $4)",
        )
        .bind(fixture.release)
        .bind(key)
        .bind(route)
        .bind(fixture.release_agent)
        .execute(pool)
        .await;
        assert_sqlstate(result, "23514");
    }

    let wrong_kind = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'wrong-kind', $2, 'executable', 'text/html')",
    )
    .bind(fixture.release)
    .bind(fixture.artifact)
    .execute(pool)
    .await;
    assert_sqlstate(wrong_kind, "23514");
    let wrong_media = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'wrong-media', $2, 'file', 'text/css')",
    )
    .bind(fixture.release)
    .bind(fixture.artifact)
    .execute(pool)
    .await;
    assert_sqlstate(wrong_media, "23503");
    let wrong_static_kind = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, content_kind, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'wrong-content', 'managed_service', $2, 'file', 'text/html')",
    )
    .bind(fixture.release)
    .bind(fixture.artifact)
    .execute(pool)
    .await;
    assert_sqlstate(wrong_static_kind, "23514");
    let wrong_managed_kind = sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, content_kind, gateway_name, route, release_agent_id)
         VALUES ($1, 'service', 'static', 'ui-service', '/wrong-content', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(wrong_managed_kind, "23514");
}

async fn assert_immutable_rows(pool: &PgPool, fixture: &ReleaseFixture) {
    let mutations = [
        sqlx::query("UPDATE release_ui_source_snapshots SET created_at = now() WHERE release_id = $1")
            .bind(fixture.release),
        sqlx::query("UPDATE release_ui_descriptors SET label = 'Changed' WHERE release_id = $1 AND ui_key = 'docs'")
            .bind(fixture.release),
        sqlx::query("UPDATE release_ui_static_files SET route = 'changed.html' WHERE release_id = $1 AND ui_key = 'docs' AND route = 'index.html'")
            .bind(fixture.release),
        sqlx::query("UPDATE release_ui_managed_services SET route = '/changed' WHERE release_id = $1 AND ui_key = 'service'")
            .bind(fixture.release),
        sqlx::query("UPDATE release_ui_api_bindings SET method = 'POST' WHERE release_id = $1 AND ui_key = 'service' AND api_key = 'health'")
            .bind(fixture.release),
    ];
    for mutation in mutations {
        assert_sqlstate(mutation.execute(pool).await, "23000");
    }
    let deletes = [
        sqlx::query("DELETE FROM release_ui_source_snapshots WHERE release_id = $1")
            .bind(fixture.release),
        sqlx::query("DELETE FROM release_ui_descriptors WHERE release_id = $1 AND ui_key = 'docs'")
            .bind(fixture.release),
        sqlx::query("DELETE FROM release_ui_static_files WHERE release_id = $1 AND ui_key = 'docs' AND route = 'index.html'")
            .bind(fixture.release),
        sqlx::query("DELETE FROM release_ui_managed_services WHERE release_id = $1 AND ui_key = 'service'")
            .bind(fixture.release),
        sqlx::query("DELETE FROM release_ui_api_bindings WHERE release_id = $1 AND ui_key = 'service' AND api_key = 'health'")
            .bind(fixture.release),
    ];
    for deletion in deletes {
        assert_sqlstate(deletion.execute(pool).await, "23000");
    }
}

async fn assert_insert_guard(pool: &PgPool, fixture: &ReleaseFixture, state: &str) {
    let source = sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(fixture.release_without_ui)
    .bind(fixture.build_without_ui)
    .bind(fixture.source_revision_without_ui)
    .execute(pool)
    .await;
    assert_sqlstate(source, "23000");

    let descriptor = sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'after-state', 'global', 'After state', 'app', 'iframe',
                 'after-state', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(fixture.release)
    .execute(pool)
    .await;
    assert_sqlstate(descriptor, "23000");

    let static_file = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', $2, $3, 'file', 'text/html')",
    )
    .bind(fixture.release)
    .bind(format!("after-{state}.html"))
    .bind(fixture.artifact)
    .execute(pool)
    .await;
    assert_sqlstate(static_file, "23000");

    let managed = sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'service', 'ui-service', $2, $3)",
    )
    .bind(fixture.release)
    .bind(format!("/after-{state}"))
    .bind(fixture.release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(managed, "23000");

    let api = sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
         VALUES ($1, 'service', $2, 'ui-service', 'GET', '/after-state', $3)",
    )
    .bind(fixture.release)
    .bind(format!("after-{state}"))
    .bind(fixture.release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(api, "23000");
}

async fn assert_rls_isolation(url: &str, fixture: &ReleaseFixture) {
    let owner = role_pool(url, "hephaestus_app", fixture.owner).await;
    let visible: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_static_files WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1)",
    )
    .bind(fixture.release)
    .fetch_one(&owner)
    .await
    .expect("authorized release UI read");
    assert_eq!(visible, (1, 3, 1, 1, 1));

    let outsider = role_pool(url, "hephaestus_app", fixture.outsider).await;
    let hidden: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_static_files WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1)",
    )
    .bind(fixture.release)
    .fetch_one(&outsider)
    .await
    .expect("unauthorized release UI read is filtered");
    assert_eq!(hidden, (0, 0, 0, 0, 0));
    let denied = sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'outsider', 'global', 'Outsider', 'app', 'iframe',
                 'outsider', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(fixture.release)
    .execute(&outsider)
    .await;
    // The draft guard runs before the INSERT policy and sees no release row
    // through outsider RLS, so it fails closed with the same integrity error.
    assert_sqlstate(denied, "23000");
}

async fn role_pool(url: &str, role: &str, actor: Uuid) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
        .expect("connect RLS test pool");
    sqlx::query("SELECT set_config('role', $1, false)")
        .bind(role)
        .execute(&pool)
        .await
        .expect("set RLS database role");
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(actor.to_string())
        .execute(&pool)
        .await
        .expect("set RLS actor");
    pool
}

async fn assert_state(pool: &PgPool, release: Uuid, expected: &str) {
    let state: String = sqlx::query_scalar("SELECT state FROM releases WHERE id = $1")
        .bind(release)
        .fetch_one(pool)
        .await
        .expect("read release state");
    assert_eq!(state, expected);
}

fn assert_sqlstate<T>(result: Result<T, sqlx::Error>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("expected PostgreSQL error {expected}"),
        Err(error) => error,
    };
    let code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(|code| code.to_string());
    assert_eq!(code.as_deref(), Some(expected));
}

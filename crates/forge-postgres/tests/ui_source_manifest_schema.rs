//! Real `PostgreSQL` coverage for immutable UI source-manifest capture records.

use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use uuid::Uuid;

const VALID_COMMIT: &str = "a";
const OTHER_COMMIT: &str = "b";

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ui_source_manifest_schema_enforces_capture_and_link_boundaries() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("SKIPPED UI source manifest schema: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::migrate!("../../migrations")
        .run(&admin)
        .await
        .expect("apply UI source manifest migration");
    let max_version: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&admin)
            .await
            .expect("read migration marker");
    assert!(max_version >= 82, "migration 0082 must be applied");

    let fixture = seed_fixture(&admin).await;
    let worker = worker_pool(&database_url).await;

    insert_valid_revision(
        &worker,
        fixture.public_repository,
        fixture.public_receive,
        fixture.public_revision,
    )
    .await;
    insert_valid_revision(
        &worker,
        fixture.private_repository,
        fixture.private_receive,
        fixture.private_revision,
    )
    .await;
    insert_invalid_oversized(&worker, &fixture).await;
    insert_invalid_non_blob(&worker, &fixture).await;

    let invalid_observation: (i64, bool) = sqlx::query_as(
        "SELECT actual_size_bytes, normalized_ui_config IS NULL
         FROM ui_source_manifest_revisions
         WHERE id = $1",
    )
    .bind(fixture.oversized_revision)
    .fetch_one(&worker)
    .await
    .expect("invalid observation");
    assert_eq!(invalid_observation.0, 262_145);
    assert!(
        invalid_observation.1,
        "invalid rows have no authoritative UI JSON"
    );

    let incomplete_gateway = sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, requires_gateways,
          normalized_ui_config, normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', true, $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("e"))
    .bind(commit("f"))
    .bind([5_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([6_u8; 32].as_slice())
    .execute(&worker)
    .await;
    assert!(
        incomplete_gateway.is_err(),
        "gateway-referencing valid rows require a complete gateway snapshot"
    );
    assert_sqlstate(incomplete_gateway, "23514");

    let oversized_gateway = sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, requires_gateways,
          normalized_ui_config, normalized_ui_hash, gateway_manifest_oid,
          gateway_actual_size_bytes, gateway_source_sha256,
          normalized_gateway_config, normalized_gateway_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', true,
                 $7, $8, $9, 1048577, $10, $11, $12)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("g"))
    .bind(commit("h"))
    .bind([7_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([8_u8; 32].as_slice())
    .bind(commit("i"))
    .bind([9_u8; 32].as_slice())
    .bind(json!({"version": 1, "routes": []}))
    .bind([10_u8; 32].as_slice())
    .execute(&worker)
    .await;
    assert!(
        oversized_gateway.is_err(),
        "valid gateway snapshots have a 1 MiB source cap"
    );
    assert_sqlstate(oversized_gateway, "23514");

    let missing_oid = sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind,
          status, diagnostics)
         VALUES ($1, $2, $3, $4, 'tree', 'invalid', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("j"))
    .bind(json!([{"code": "ui_manifest_missing_oid"}]))
    .execute(&worker)
    .await;
    assert!(
        missing_oid.is_err(),
        "every present entry must retain its OID"
    );
    assert_sqlstate(missing_oid, "23502");

    let visible_valid: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND status = 'valid'",
    )
    .bind(fixture.public_repository)
    .fetch_one(&admin)
    .await
    .expect("valid source revision");
    assert_eq!(visible_valid, 1);

    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM build_request_ui_source_manifests
         WHERE build_request_id = $1",
    )
    .bind(fixture.public_build)
    .fetch_one(&worker)
    .await
    .expect("valid source link");
    assert_eq!(linked, 0, "link is added after the shape checks");

    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.public_build)
    .bind(fixture.public_repository)
    .bind(commit(VALID_COMMIT))
    .bind(fixture.public_revision)
    .execute(&worker)
    .await
    .expect("link valid source revision");

    let invalid_link = sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.invalid_build)
    .bind(fixture.public_repository)
    .bind(commit("c"))
    .bind(fixture.oversized_revision)
    .execute(&worker)
    .await;
    assert!(invalid_link.is_err(), "invalid source must not be linkable");
    assert_sqlstate(invalid_link, "23503");

    let cross_repository_link = sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.private_build)
    .bind(fixture.public_repository)
    .bind(commit(VALID_COMMIT))
    .bind(fixture.public_revision)
    .execute(&worker)
    .await;
    assert!(
        cross_repository_link.is_err(),
        "cross-repository link must fail"
    );
    assert_sqlstate(cross_repository_link, "23503");

    let cross_commit_link = sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.other_commit_build)
    .bind(fixture.public_repository)
    .bind(commit(OTHER_COMMIT))
    .bind(fixture.public_revision)
    .execute(&worker)
    .await;
    assert!(cross_commit_link.is_err(), "cross-commit link must fail");
    assert_sqlstate(cross_commit_link, "23503");

    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.second_public_build)
    .bind(fixture.public_repository)
    .bind(commit(VALID_COMMIT))
    .bind(fixture.public_revision)
    .execute(&worker)
    .await
    .expect("second matching build may share source snapshot");

    let immutable_update = sqlx::query(
        "UPDATE ui_source_manifest_revisions
         SET diagnostics = '[{\"code\":\"changed\"}]'::jsonb
         WHERE id = $1",
    )
    .bind(fixture.public_revision)
    .execute(&admin)
    .await;
    assert!(
        immutable_update.is_err(),
        "source revisions must be immutable"
    );

    let immutable_delete =
        sqlx::query("DELETE FROM build_request_ui_source_manifests WHERE build_request_id = $1")
            .bind(fixture.public_build)
            .execute(&admin)
            .await;
    assert!(immutable_delete.is_err(), "source links must be immutable");

    let app = app_pool(&database_url).await;
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(fixture.outsider_user.to_string())
        .execute(&app)
        .await
        .expect("set application actor");
    let app_visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1",
    )
    .bind(fixture.public_repository)
    .fetch_one(&app)
    .await
    .expect("public repository source read");
    assert_eq!(app_visible, 3, "public repository rows are readable");
    let private_visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1",
    )
    .bind(fixture.private_repository)
    .fetch_one(&app)
    .await
    .expect("private repository source read");
    assert_eq!(private_visible, 0, "private repository rows stay isolated");

    let unauthorized_insert = sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 3, $6, 'valid', $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("f"))
    .bind(commit("d"))
    .bind([3_u8; 32].as_slice())
    .bind(json!({"version": 1}))
    .bind([4_u8; 32].as_slice())
    .execute(&app)
    .await;
    assert!(
        unauthorized_insert.is_err(),
        "public read must not grant write"
    );
    assert_sqlstate(unauthorized_insert, "42501");

    let unauthorized_link = sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.private_build)
    .bind(fixture.private_repository)
    .bind(commit(VALID_COMMIT))
    .bind(fixture.private_revision)
    .execute(&app)
    .await;
    assert!(
        unauthorized_link.is_err(),
        "private link requires build access"
    );
    assert_sqlstate(unauthorized_link, "42501");
}

fn assert_sqlstate<T>(result: Result<T, sqlx::Error>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("expected a database constraint or RLS error"),
        Err(error) => error,
    };
    let actual = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(|code| code.to_string());
    assert_eq!(actual.as_deref(), Some(expected));
}

#[derive(Clone, Copy)]
struct Fixture {
    public_repository: Uuid,
    private_repository: Uuid,
    public_receive: Uuid,
    private_receive: Uuid,
    public_build: Uuid,
    second_public_build: Uuid,
    invalid_build: Uuid,
    private_build: Uuid,
    other_commit_build: Uuid,
    public_revision: Uuid,
    private_revision: Uuid,
    oversized_revision: Uuid,
    outsider_user: Uuid,
}

// Keep the disposable database fixture together so every foreign-key and RLS
// identity used by this schema test is visible in one setup path.
#[allow(clippy::too_many_lines)]
async fn seed_fixture(pool: &PgPool) -> Fixture {
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let private_project = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let outsider_user = Uuid::new_v4();
    let public_repository = Uuid::new_v4();
    let private_repository = Uuid::new_v4();
    let public_receive = Uuid::new_v4();
    let private_receive = Uuid::new_v4();
    let public_build = Uuid::new_v4();
    let second_public_build = Uuid::new_v4();
    let invalid_build = Uuid::new_v4();
    let private_build = Uuid::new_v4();
    let other_commit_build = Uuid::new_v4();
    let public_revision = Uuid::new_v4();
    let private_revision = Uuid::new_v4();
    let oversized_revision = Uuid::new_v4();

    for (id, name) in [
        (owner, "UI schema owner"),
        (outsider_user, "UI schema outsider"),
    ] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .expect("user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'ui-schema-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(owner)
    .execute(pool)
    .await
    .expect("organization owner");
    for (id, name) in [
        (project, "ui-schema-project"),
        (private_project, "ui-schema-private"),
    ] {
        sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(organization)
            .bind(name)
            .execute(pool)
            .await
            .expect("project");
    }
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project)
        .bind(owner)
        .execute(pool)
        .await
        .expect("project maintainer");
    for (id, project_id, name, public) in [
        (public_repository, project, "ui-schema-public", true),
        (
            private_repository,
            private_project,
            "ui-schema-private",
            false,
        ),
    ] {
        sqlx::query(
            "INSERT INTO repositories (id, project_id, name, is_public)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(project_id)
        .bind(name)
        .bind(public)
        .execute(pool)
        .await
        .expect("repository");
    }
    for (id, repository_id) in [
        (public_receive, public_repository),
        (private_receive, private_repository),
    ] {
        sqlx::query(
            "INSERT INTO git_receives
             (id, repository_id, principal, status, accepted_at)
             VALUES ($1, $2, 'ui-schema-test', 'accepted', now())",
        )
        .bind(id)
        .bind(repository_id)
        .execute(pool)
        .await
        .expect("receive");
    }
    for (id, repository_id, source_commit, receive_id) in [
        (
            public_build,
            public_repository,
            commit(VALID_COMMIT),
            public_receive,
        ),
        (
            invalid_build,
            public_repository,
            commit("c"),
            public_receive,
        ),
        (
            private_build,
            private_repository,
            commit(VALID_COMMIT),
            private_receive,
        ),
        (
            other_commit_build,
            public_repository,
            commit(OTHER_COMMIT),
            public_receive,
        ),
    ] {
        sqlx::query(
            "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, origin_receive_id,
              build_definition_hash, state)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'queued')",
        )
        .bind(id)
        .bind(repository_id)
        .bind(source_commit)
        .bind(receive_id)
        .bind([8_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("build request");
    }
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'queued')",
    )
    .bind(second_public_build)
    .bind(public_repository)
    .bind(commit(VALID_COMMIT))
    .bind(public_receive)
    .bind([9_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("second matching build request");
    Fixture {
        public_repository,
        private_repository,
        public_receive,
        private_receive,
        public_build,
        second_public_build,
        invalid_build,
        private_build,
        other_commit_build,
        public_revision,
        private_revision,
        oversized_revision,
        outsider_user,
    }
}

async fn insert_valid_revision(pool: &PgPool, repository: Uuid, receive: Uuid, revision: Uuid) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', $7, $8)",
    )
    .bind(revision)
    .bind(repository)
    .bind(receive)
    .bind(commit(VALID_COMMIT))
    .bind(commit("c"))
    .bind([1_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([2_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("valid source revision");
}

async fn insert_invalid_oversized(pool: &PgPool, fixture: &Fixture) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, status, diagnostics)
         VALUES ($1, $2, $3, $4, 'blob', $5, 262145, 'invalid', $6)",
    )
    .bind(fixture.oversized_revision)
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("c"))
    .bind(commit("e"))
    .bind(json!([{"code": "ui_manifest_too_large"}]))
    .execute(pool)
    .await
    .expect("oversized invalid source revision");
}

async fn insert_invalid_non_blob(pool: &PgPool, fixture: &Fixture) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          status, diagnostics)
         VALUES ($1, $2, $3, $4, 'symlink', $5, 'invalid', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("d"))
    .bind(commit("e"))
    .bind(json!([{"code": "ui_manifest_not_regular_blob"}]))
    .execute(pool)
    .await
    .expect("non-blob invalid source revision");
}

async fn worker_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect worker role")
}

async fn app_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
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

fn commit(prefix: &str) -> String {
    format!("{prefix}{}", "0".repeat(39))
}

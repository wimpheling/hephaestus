//! External real-PostgreSQL matrix for the 0089 draft.
//!
//! This file is intentionally outside the repository.  Copy it into the
//! release-postgres schema-test target only after migration 0089 is reviewed.
//! It is source-only here and has not been compiled or run.

use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use tokio::time::timeout;
use uuid::Uuid;

use identity_domain::{BrowserSessionId, RequestId, UserId};
use release_domain::{
    UiInstallationGenerationId, UiInstallationId,
    ui_browser::{UiBrowserHandoffSecret, UiBrowserRoute, UiBrowserSessionSecret},
};
use release_postgres::PgUiBrowserSessionStore;
use release_service::{CreateUiBrowserHandoff, ExchangeUiBrowserHandoff, UiBrowserHandoffError};

const EXPECTED_MIGRATION: i64 = 89;

#[derive(Clone, Copy)]
struct Fixture {
    actor: Uuid,
    outsider: Uuid,
    organization: Uuid,
    other_organization: Uuid,
    parent_session: Uuid,
    outsider_parent_session: Uuid,
    installation: Uuid,
    other_installation: Uuid,
    generation: Uuid,
    other_generation: Uuid,
    global_installation: Uuid,
    global_generation: Uuid,
    repository_installation: Uuid,
    repository_generation: Uuid,
    route: &'static str,
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_schema_matrix_enforces_bindings_lifecycle_timing_and_roles() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser schema: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0089");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker");
    assert!(max_migration >= EXPECTED_MIGRATION);

    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    assert_role(&worker, "hephaestus_worker", false, true).await;
    assert_role(&app, "hephaestus_app", false, false).await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;

    assert_digest_constraints(&worker, &fixture).await;
    assert_wrong_actor_binding(&worker, &fixture).await;
    assert_wrong_organization_binding(&worker, &fixture).await;
    assert_wrong_installation_generation_binding(&worker, &fixture).await;
    assert_wrong_child_binding(&worker, &fixture).await;
    assert_initial_consumption_is_rejected(&worker, &fixture).await;
    assert_child_issue_interval_and_parent_cap(&worker, &fixture).await;
    assert_child_only_commit_rolls_back(&worker, &fixture).await;
    assert_consume_only_is_rejected(&worker, &fixture).await;
    assert_commit_and_consume_is_one_time(&worker, &fixture).await;
    assert_expiry_and_twelve_hour_cap(&worker, &fixture).await;
    assert_application_role_is_denied(&app, &fixture).await;

    println!(
        "REAL_UI_BROWSER_SCHEMA=1 migration={max_migration} digest=1 binding=1 initial_consumed=1 child_interval=1 transaction=1 one_time=1 expiry=1 role_denial=1"
    );
}

// This is deliberately self-contained. It follows the publication-parent
// order from ui_installation_schema.rs, then adds canonical identity sessions
// and two valid project installations/generations. Every negative case below
// therefore reaches the 0089 invariant under test instead of an unrelated FK.
#[allow(clippy::too_many_lines)]
async fn seed_fixture_reusing_installation_helpers(worker: &PgPool) -> Fixture {
    let actor = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let other_organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let receive = Uuid::new_v4();
    let build = Uuid::new_v4();
    let source_revision = Uuid::new_v4();
    let release = Uuid::new_v4();
    let parent_session = Uuid::new_v4();
    let outsider_parent_session = Uuid::new_v4();
    let installation = Uuid::new_v4();
    let other_installation = Uuid::new_v4();
    let generation = Uuid::new_v4();
    let other_generation = Uuid::new_v4();
    let global_installation = Uuid::new_v4();
    let global_generation = Uuid::new_v4();
    let repository_installation = Uuid::new_v4();
    let repository_generation = Uuid::new_v4();
    let request_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO users (id, display_name) VALUES ($1, 'UI browser actor'), ($2, 'UI browser outsider')",
    )
    .bind(actor)
    .bind(outsider)
    .execute(worker)
    .await
    .expect("seed users");
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(actor.to_string())
        .execute(worker)
        .await
        .expect("set fixture actor");
    sqlx::query(
        "INSERT INTO organizations (id, name) VALUES ($1, 'UI browser org'), ($2, 'Other org')",
    )
    .bind(organization)
    .bind(other_organization)
    .execute(worker)
    .await
    .expect("seed organizations");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'ui-browser')")
        .bind(project)
        .bind(organization)
        .execute(worker)
        .await
        .expect("seed project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'ui-browser-repository')",
    )
    .bind(repository)
    .bind(project)
    .execute(worker)
    .await
    .expect("seed repository");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'ui-browser-schema', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository)
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed accepted receive");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'succeeded', $6)",
    )
    .bind(build)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(receive)
    .bind(vec![1_u8; 32])
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed build request");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config, normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', '{}'::jsonb, $7)",
    )
    .bind(source_revision)
    .bind(repository)
    .bind(receive)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(worker)
    .await
    .expect("seed source manifest revision");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit, source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(build)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(source_revision)
    .execute(worker)
    .await
    .expect("link source manifest revision");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         VALUES ($1, $2, 'ui-browser-v1', $3, 'refs/heads/main', $4, $5,
                 '{}'::jsonb, $6, $7, 'draft')",
    )
    .bind(release)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(build)
    .bind(vec![1_u8; 32])
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(worker)
    .await
    .expect("seed published release");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(release)
    .bind(build)
    .bind(source_revision)
    .execute(worker)
    .await
    .expect("seed release UI snapshot");
    for (ui_key, route_base, scope) in [
        ("schema-ui", "schema-ui", "project"),
        ("schema-ui-two", "schema-ui-two", "project"),
        ("schema-global", "schema-global", "global"),
        ("schema-repository", "schema-repository", "repository"),
    ] {
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, $2, $5, $3, 'app', 'iframe', $4,
                     'index.html', 1, 'no_store', 'static')",
        )
        .bind(release)
        .bind(ui_key)
        .bind(ui_key)
        .bind(route_base)
        .bind(scope)
        .execute(worker)
        .await
        .expect("seed release UI descriptor");
    }
    sqlx::query(
        "UPDATE releases SET state = 'published', published_at = now(),
                publication_actor_id = $2 WHERE id = $1",
    )
    .bind(release)
    .bind(actor)
    .execute(worker)
    .await
    .expect("publish release after descriptors");

    insert_canonical_session(worker, parent_session, actor, request_id, 20).await;
    insert_canonical_session(
        worker,
        outsider_parent_session,
        outsider,
        Uuid::new_v4(),
        20,
    )
    .await;
    seed_project_installation(
        worker,
        installation,
        generation,
        release,
        "schema-ui",
        actor,
        project,
    )
    .await;
    seed_global_installation(
        worker,
        global_installation,
        global_generation,
        release,
        "schema-global",
        actor,
        organization,
    )
    .await;
    seed_repository_installation(
        worker,
        repository_installation,
        repository_generation,
        release,
        "schema-repository",
        actor,
        project,
        repository,
    )
    .await;
    seed_project_installation(
        worker,
        other_installation,
        other_generation,
        release,
        "schema-ui-two",
        actor,
        project,
    )
    .await;

    Fixture {
        actor,
        outsider,
        organization,
        other_organization,
        parent_session,
        outsider_parent_session,
        installation,
        other_installation,
        generation,
        other_generation,
        global_installation,
        global_generation,
        repository_installation,
        repository_generation,
        route: "ui",
    }
}

async fn insert_canonical_session(
    worker: &PgPool,
    session_id: Uuid,
    user_id: Uuid,
    request_id: Uuid,
    expiry_hours: i64,
) {
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + ($7::int * interval '1 hour'))",
    )
    .bind(session_id)
    .bind(digest(200))
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(digest(201))
    .bind(user_id)
    .bind(expiry_hours)
    .execute(worker)
    .await
    .expect("seed canonical human browser session");
}

async fn seed_project_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    project: Uuid,
) {
    let mut tx = worker.begin().await.expect("begin installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, 'project', $3, 'enabled', $4, $5)",
    )
    .bind(installation_id)
    .bind(project)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed project UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'project')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed project UI generation");
    tx.commit().await.expect("commit project UI installation");
}

async fn seed_global_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    organization: Uuid,
) {
    let mut tx = worker
        .begin()
        .await
        .expect("begin global installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4, $5)",
    )
    .bind(installation_id)
    .bind(organization)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed global UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'global')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed global UI generation");
    tx.commit().await.expect("commit global UI installation");
}

// Keep every repository owner and generation column explicit in this fixture
// so the scope and composite-FK proof stays readable at the SQL boundary.
#[allow(clippy::too_many_arguments)]
async fn seed_repository_installation(
    worker: &PgPool,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
    actor: Uuid,
    project: Uuid,
    repository: Uuid,
) {
    let mut tx = worker
        .begin()
        .await
        .expect("begin repository installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'repository', $4, 'enabled', $5, $6)",
    )
    .bind(installation_id)
    .bind(project)
    .bind(repository)
    .bind(ui_key)
    .bind(generation_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .expect("seed repository UI installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'repository')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed repository UI generation");
    tx.commit()
        .await
        .expect("commit repository UI installation");
}

fn digest(seed: u8) -> Vec<u8> {
    let mut value = vec![seed; 32];
    value[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    value
}

async fn assert_digest_constraints(worker: &PgPool, fixture: &Fixture) {
    let handoff = Uuid::new_v4();
    let bad_lengths = [vec![0_u8; 31], vec![0_u8; 33]];
    for digest in bad_lengths {
        assert_rejected_code(
            sqlx::query(
                "INSERT INTO ui_browser_handoffs
                 (id, handoff_digest, request_id, actor_id, parent_session_id,
                  installation_id, generation_id, organization_id, route,
                  issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                         statement_timestamp(), statement_timestamp() + interval '60 seconds')",
            )
            .bind(handoff)
            .bind(&digest)
            .bind(Uuid::new_v4())
            .bind(fixture.actor)
            .bind(fixture.parent_session)
            .bind(fixture.installation)
            .bind(fixture.generation)
            .bind(fixture.organization)
            .bind(fixture.route)
            .execute(worker),
            "23514",
            "handoff digest must be exactly 32 bytes",
        )
        .await;
    }
    let absent: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1")
            .bind(vec![0_u8; 32])
            .fetch_optional(worker)
            .await
            .expect("worker can perform a digest lookup for this matrix");
    assert!(
        absent.is_none(),
        "an unrelated digest must not authenticate"
    );
}

async fn assert_wrong_actor_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query(
            "INSERT INTO ui_browser_handoffs
             (id, handoff_digest, request_id, actor_id, parent_session_id,
              installation_id, generation_id, organization_id, route,
              issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     statement_timestamp(), statement_timestamp() + interval '60 seconds')",
        )
        .bind(Uuid::new_v4())
        .bind(digest(1))
        .bind(Uuid::new_v4())
        .bind(fixture.outsider)
        .bind(fixture.parent_session)
        .bind(fixture.installation)
        .bind(fixture.generation)
        .bind(fixture.organization)
        .bind(fixture.route)
        .execute(worker),
        "23000",
        "actor must own the canonical parent session",
    )
    .await;
}

async fn assert_wrong_organization_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.other_organization,
            fixture.installation,
            fixture.generation,
            digest(2),
        ),
        "23000",
        "organization must be derived from the installation target",
    )
    .await;
}

async fn assert_wrong_installation_generation_binding(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.installation,
            fixture.other_generation,
            digest(3),
        ),
        "23503",
        "generation and installation must satisfy the composite FK",
    )
    .await;
    assert_rejected_code(
        insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.other_installation,
            fixture.generation,
            digest(4),
        ),
        "23503",
        "generation must belong to the selected installation",
    )
    .await;
}

async fn assert_wrong_child_binding(worker: &PgPool, fixture: &Fixture) {
    let cases = [
        (
            fixture.outsider_parent_session,
            fixture.installation,
            fixture.generation,
            fixture.organization,
            "parent actor/session binding",
        ),
        (
            fixture.parent_session,
            fixture.installation,
            fixture.generation,
            fixture.other_organization,
            "child organization binding",
        ),
        (
            fixture.parent_session,
            fixture.other_installation,
            fixture.other_generation,
            fixture.organization,
            "child installation/generation binding",
        ),
    ];
    for (parent_session, installation, generation, organization, reason) in cases {
        let handoff = insert_handoff(
            worker,
            fixture,
            fixture.organization,
            fixture.installation,
            fixture.generation,
            digest(20),
        )
        .await
        .expect("insert handoff for child binding case");
        assert_rejected_code(
            insert_child_with_binding(
                worker,
                fixture,
                handoff,
                parent_session,
                installation,
                generation,
                organization,
                digest(21),
            ),
            "23000",
            reason,
        )
        .await;
    }
}

async fn assert_initial_consumption_is_rejected(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query(
            "INSERT INTO ui_browser_handoffs
             (id, handoff_digest, request_id, actor_id, parent_session_id,
              installation_id, generation_id, organization_id, route,
              issued_at, expires_at, consumed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     statement_timestamp(), statement_timestamp() + interval '60 seconds',
                     statement_timestamp())",
        )
        .bind(Uuid::new_v4())
        .bind(digest(5))
        .bind(Uuid::new_v4())
        .bind(fixture.actor)
        .bind(fixture.parent_session)
        .bind(fixture.installation)
        .bind(fixture.generation)
        .bind(fixture.organization)
        .bind(fixture.route)
        .execute(worker),
        "23000",
        "consumed_at is a transition and cannot be supplied on INSERT",
    )
    .await;
}

async fn assert_child_issue_interval_and_parent_cap(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(6),
    )
    .await
    .expect("insert valid handoff");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "60 seconds", "60 seconds"),
        "23000",
        "child issue at handoff expiry must fail",
    )
    .await;
    let before_child_handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(15),
    )
    .await
    .expect("insert handoff for child-issue ordering");
    let mut tx = worker
        .begin()
        .await
        .expect("begin child-issue ordering transaction");
    insert_child_tx(
        &mut tx,
        fixture,
        before_child_handoff,
        "30 seconds",
        "1 hour",
    )
    .await
    .expect("insert future-issued child");
    let before_child_error = sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = issued_at + interval '1 second' WHERE id = $1",
    )
    .bind(before_child_handoff)
    .execute(&mut *tx)
    .await
    .expect_err("consumption before child issue should fail");
    assert_database_code(
        &before_child_error,
        "23000",
        "consumption before child issue",
    );
    tx.rollback()
        .await
        .expect("rollback child-issue ordering rejection");
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(7),
    )
    .await
    .expect("insert second valid handoff");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "1 second", "13 hours"),
        "23514",
        "child expiry may not exceed the twelve-hour cap or parent expiry",
    )
    .await;
}

async fn assert_child_only_commit_rolls_back(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(8),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before deferred rollback");
    let commit_error = tx
        .commit()
        .await
        .expect_err("deferred consumed trigger must reject child-only commit");
    assert_database_code(&commit_error, "23000", "child-only commit");
    let consumed: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(handoff)
            .fetch_one(worker)
            .await
            .expect("handoff remains after child-only rollback");
    assert!(consumed.is_none());
    let child_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(handoff)
            .fetch_one(worker)
            .await
            .expect("count child rows after rollback");
    assert_eq!(
        child_count, 0,
        "child-only rollback must leave no child row"
    );
}

async fn assert_consume_only_is_rejected(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(9),
    )
    .await
    .expect("insert valid handoff");
    assert_rejected_code(
        sqlx::query(
            "UPDATE ui_browser_handoffs
             SET consumed_at = statement_timestamp() WHERE id = $1",
        )
        .bind(handoff)
        .execute(worker),
        "23000",
        "handoff cannot be consumed without a child",
    )
    .await;
}

async fn assert_commit_and_consume_is_one_time(worker: &PgPool, fixture: &Fixture) {
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(10),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before one-time consume");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume matching child in same transaction");
    tx.commit()
        .await
        .expect("commit child and consume atomically");
    assert_rejected_code(
        insert_child(worker, fixture, handoff, "0 seconds", "1 hour"),
        "23000",
        "unique handoff_id prevents a second child",
    )
    .await;
    assert_rejected_code(
        sqlx::query(
            "UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1",
        )
        .bind(handoff)
        .execute(worker),
        "23000",
        "a consumed handoff cannot be reconsumed",
    )
    .await;
}

async fn assert_expiry_and_twelve_hour_cap(worker: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        insert_handoff_with_times(worker, fixture, digest(11), "-61 seconds", "0 seconds"),
        "23514",
        "handoff lifetime must be exactly sixty seconds",
    )
    .await;
    let handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(12),
    )
    .await
    .expect("insert valid handoff");
    let mut tx = worker.begin().await.expect("begin worker transaction");
    insert_child_tx(&mut tx, fixture, handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before expiry assertion");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume child before expiry assertion");
    tx.commit().await.expect("commit valid child and consume");
    let expired_handoff = insert_handoff(
        worker,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(14),
    )
    .await
    .expect("insert second handoff for expiry timestamp");
    let mut tx = worker.begin().await.expect("begin expiry transaction");
    insert_child_tx(&mut tx, fixture, expired_handoff, "0 seconds", "1 hour")
        .await
        .expect("insert child before expiry rejection");
    let expiry_error = sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = issued_at + interval '60 seconds' WHERE id = $1",
    )
    .bind(expired_handoff)
    .execute(&mut *tx)
    .await
    .expect_err("consumption at expiry should fail before commit");
    assert_database_code(&expiry_error, "23000", "consumption at expiry");
    tx.rollback().await.expect("rollback expiry rejection");
}

async fn assert_application_role_is_denied(app: &PgPool, fixture: &Fixture) {
    assert_rejected_code(
        sqlx::query("SELECT * FROM ui_browser_handoffs").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("SELECT * FROM ui_browser_sessions").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("INSERT INTO ui_browser_handoffs DEFAULT VALUES").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("INSERT INTO ui_browser_sessions DEFAULT VALUES").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = now()").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("DELETE FROM ui_browser_handoffs").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    assert_rejected_code(
        sqlx::query("DELETE FROM ui_browser_sessions").execute(app),
        "42501",
        "application role must be denied",
    )
    .await;
    let _ = fixture;
}

// The following helpers deliberately centralize the statement-time expressions
// so every matrix case remains comparable with the draft trigger checks.
async fn insert_handoff(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    digest: Vec<u8>,
) -> Result<Uuid, sqlx::Error> {
    insert_handoff_with_times_for(
        pool,
        fixture,
        organization,
        installation,
        generation,
        digest,
        "0 seconds",
        "60 seconds",
    )
    .await
}

async fn insert_handoff_with_times(
    pool: &PgPool,
    fixture: &Fixture,
    digest: Vec<u8>,
    issued_at: &str,
    expires_at: &str,
) -> Result<Uuid, sqlx::Error> {
    insert_handoff_with_times_for(
        pool,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest,
        issued_at,
        expires_at,
    )
    .await
}

// The matrix keeps each persisted binding dimension visible at the call site.
#[allow(clippy::too_many_arguments)]
async fn insert_handoff_with_times_for(
    pool: &PgPool,
    fixture: &Fixture,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    digest: Vec<u8>,
    issued_at: &str,
    expires_at: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp() + $10::interval,
                 statement_timestamp() + $11::interval)",
    )
    .bind(id)
    .bind(digest)
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(installation)
    .bind(generation)
    .bind(organization)
    .bind(fixture.route)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| id)
}

async fn insert_child(
    pool: &PgPool,
    fixture: &Fixture,
    handoff: Uuid,
    issued_at: &str,
    expires_at: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    insert_child_tx(&mut tx, fixture, handoff, issued_at, expires_at).await?;
    tx.commit().await
}

// The negative cases intentionally override each durable binding independently.
#[allow(clippy::too_many_arguments)]
async fn insert_child_with_binding(
    pool: &PgPool,
    fixture: &Fixture,
    handoff: Uuid,
    parent_session: Uuid,
    installation: Uuid,
    generation: Uuid,
    organization: Uuid,
    session_digest: Vec<u8>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, $4, $5, $6, $7, $8,
                handoff.issued_at, handoff.issued_at + interval '1 hour'
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $9",
    )
    .bind(Uuid::new_v4())
    .bind(session_digest)
    .bind(Uuid::new_v4())
    .bind(parent_session)
    .bind(installation)
    .bind(generation)
    .bind(organization)
    .bind(fixture.route)
    .bind(handoff)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

async fn insert_child_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    _fixture: &Fixture,
    handoff: Uuid,
    issued_at: &str,
    expires_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at + $5::interval,
                handoff.issued_at + $6::interval
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(Uuid::new_v4())
    .bind(digest(13))
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(issued_at)
    .bind(expires_at)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

async fn assert_rejected_code<F, T>(operation: F, expected_code: &str, reason: &str)
where
    F: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    let result = timeout(Duration::from_secs(5), operation)
        .await
        .expect("schema operation did not finish");
    let error = match result {
        Ok(_) => panic!("schema operation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_database_code(&error, expected_code, reason);
}

fn assert_database_code(error: &sqlx::Error, expected_code: &str, reason: &str) {
    let actual_code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code);
    assert_eq!(
        actual_code.as_deref(),
        Some(expected_code),
        "unexpected SQLSTATE for {reason}"
    );
}

// These helpers match the existing release schema tests. The test URL's login
// role must be allowed to SET ROLE to each restricted disposable-db role.
async fn role_pool(database_url: &str, role: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role.clone())
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(format!("ui-browser-{role}"))
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
                        .bind(Uuid::nil().to_string())
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect restricted PostgreSQL role")
}

async fn wait_for_named_lock_waiter(admin: &PgPool, application_name: &str, blocker_pid: i32) {
    for _ in 0..200 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE application_name = $1
               AND wait_event_type = 'Lock'
               AND $2 = ANY(pg_blocking_pids(pid))",
        )
        .bind(application_name)
        .bind(blocker_pid)
        .fetch_one(admin)
        .await
        .expect("inspect issuance lock waiter");
        if waiting > 0 {
            println!(
                "REAL_UI_BROWSER_ISSUE_LOCK_BARRIER=1 application_name={application_name} blocker_pid={blocker_pid}"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for issuance lock waiter {application_name}");
}

async fn wait_for_named_exchange_lock_waiter(
    admin: &PgPool,
    application_name: &str,
    blocker_pid: i32,
) {
    for _ in 0..1000 {
        let waiting: i64 = sqlx::query_scalar(
            "WITH RECURSIVE blocker_chain(pid, blocking_pid, depth) AS (
                 SELECT activity.pid, unnest(pg_blocking_pids(activity.pid)), 0
                 FROM pg_stat_activity AS activity
                 WHERE activity.application_name = $1
                   AND activity.wait_event_type = 'Lock'
                 UNION ALL
                 SELECT blocker_chain.pid,
                        unnest(pg_blocking_pids(blocker_chain.blocking_pid)),
                        blocker_chain.depth + 1
                 FROM blocker_chain
                 WHERE blocker_chain.depth < 4
             )
             SELECT count(*) FROM blocker_chain WHERE blocking_pid = $2",
        )
        .bind(application_name)
        .bind(blocker_pid)
        .fetch_one(admin)
        .await
        .expect("inspect exchange lock waiter");
        if waiting > 0 {
            println!(
                "REAL_UI_BROWSER_EXCHANGE_LOCK_BARRIER=1 application_name={application_name} blocker_pid={blocker_pid}"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for exchange lock waiter {application_name}");
}

async fn assert_role(pool: &PgPool, expected: &str, superuser: bool, bypass_rls: bool) {
    let row: (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .expect("read PostgreSQL role identity");
    assert_eq!(row, (expected.to_owned(), superuser, bypass_rls));
}

/// Exercises the first adapter slice against the complete 0089 fixture. The
/// schema matrix above remains the owner of SQLSTATE and lifecycle checks.
#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_issue_binds_current_authority_and_fresh_expiry() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser issue: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0089");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let store = PgUiBrowserSessionStore::new(worker.clone(), app);
    let barrier_app = role_pool(&database_url, "hephaestus_app").await;
    let actor = UserId::from_uuid(fixture.actor);
    let parent = BrowserSessionId::from_uuid(fixture.parent_session);
    let installation = UiInstallationId::from_uuid(fixture.installation);
    let generation = UiInstallationGenerationId::from_uuid(fixture.generation);
    let route = UiBrowserRoute::parse("schema-ui").expect("published route base");
    let request_id = RequestId::new();
    let secret = UiBrowserHandoffSecret::random();
    let expected_digest = secret.digest().as_bytes();
    let created = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret,
        })
        .await
        .expect("active actor may issue for current generation");
    assert_eq!(created.organization_id.as_uuid(), fixture.organization);
    assert_eq!(created.route, route);

    let stored: (
        Vec<u8>,
        Uuid,
        Uuid,
        Uuid,
        Uuid,
        String,
        time::OffsetDateTime,
        time::OffsetDateTime,
    ) = sqlx::query_as(
        "SELECT handoff_digest, request_id, actor_id, parent_session_id,
                    organization_id, route, issued_at, expires_at
             FROM ui_browser_handoffs WHERE id = $1",
    )
    .bind(created.handoff_id.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("read issued handoff metadata");
    assert_eq!(
        stored.0, expected_digest,
        "stored digest matches the secret"
    );
    assert_eq!(stored.1, request_id.as_uuid());
    assert_eq!(stored.2, fixture.actor);
    assert_eq!(stored.3, fixture.parent_session);
    assert_eq!(stored.4, fixture.organization);
    assert_eq!(stored.5, "schema-ui");
    assert_eq!(stored.7 - stored.6, time::Duration::seconds(60));

    for (installation_id, generation_id, route_text) in [
        (
            fixture.global_installation,
            fixture.global_generation,
            "schema-global",
        ),
        (
            fixture.repository_installation,
            fixture.repository_generation,
            "schema-repository",
        ),
    ] {
        let scope_secret = UiBrowserHandoffSecret::random();
        let scope_digest = scope_secret.digest().as_bytes();
        let scope_created = store
            .create_ui_browser_handoff(CreateUiBrowserHandoff {
                request_id: RequestId::new(),
                actor_id: actor,
                parent_session_id: parent,
                installation_id: UiInstallationId::from_uuid(installation_id),
                generation_id: UiInstallationGenerationId::from_uuid(generation_id),
                route: UiBrowserRoute::parse(route_text).expect("scope route"),
                secret: scope_secret,
            })
            .await
            .expect("active actor may issue for every owner scope");
        assert_eq!(
            scope_created.organization_id.as_uuid(),
            fixture.organization
        );
        let stored_scope_digest: Vec<u8> =
            sqlx::query_scalar("SELECT handoff_digest FROM ui_browser_handoffs WHERE id = $1")
                .bind(scope_created.handoff_id.as_uuid())
                .fetch_one(&worker)
                .await
                .expect("read scope handoff digest");
        assert_eq!(stored_scope_digest, scope_digest);
    }

    let baseline: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count issued handoffs");

    let expiring_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp(),
                 statement_timestamp() + interval '1 second')",
    )
    .bind(expiring_parent)
    .bind(digest(222))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(223))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed lock-barrier parent");
    let mut account_lock = bootstrap.begin().await.expect("begin account lock barrier");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *account_lock)
        .await
        .expect("read account lock barrier PID");
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(fixture.actor)
        .fetch_one(&mut *account_lock)
        .await
        .expect("hold actor account lock barrier");
    let barrier_store = PgUiBrowserSessionStore::new(worker.clone(), barrier_app);
    let barrier_route = route.clone();
    let barrier_task = tokio::spawn(async move {
        barrier_store
            .create_ui_browser_handoff(CreateUiBrowserHandoff {
                request_id: RequestId::new(),
                actor_id: actor,
                parent_session_id: BrowserSessionId::from_uuid(expiring_parent),
                installation_id: installation,
                generation_id: generation,
                route: barrier_route,
                secret: UiBrowserHandoffSecret::random(),
            })
            .await
    });
    wait_for_named_lock_waiter(&bootstrap, "ui-browser-hephaestus_worker", blocker_pid).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    account_lock
        .commit()
        .await
        .expect("release account lock barrier");
    assert_eq!(
        barrier_task.await.expect("expiry barrier task"),
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    let after_barrier: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count after expiry barrier");
    assert_eq!(after_barrier, baseline);

    let route_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("arbitrary").expect("safe undeclared route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(route_denied, Err(UiBrowserHandoffError::InvalidRoute));
    let after_route: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count after route denial");
    assert_eq!(after_route, baseline);

    let revoked_parent_id = Uuid::new_v4();
    insert_canonical_session(
        &worker,
        revoked_parent_id,
        fixture.actor,
        Uuid::new_v4(),
        20,
    )
    .await;
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'administrative'
         WHERE id = $1",
    )
    .bind(revoked_parent_id)
    .execute(&worker)
    .await
    .expect("revoke parent session");
    let revoked_parent = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(revoked_parent_id),
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(revoked_parent, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend actor account");
    let inactive_actor = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(inactive_actor, Err(UiBrowserHandoffError::PermissionDenied));
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore actor account");

    let future_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp() + interval '1 hour',
                 statement_timestamp() + interval '2 hours')",
    )
    .bind(future_parent)
    .bind(digest(224))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(225))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed future-issued parent");
    let future_issued = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(future_parent),
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(future_issued, Err(UiBrowserHandoffError::PermissionDenied));

    let actor_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: UserId::from_uuid(fixture.outsider),
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(actor_denied, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("disable installation for denial case");
    let disabled_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(
        disabled_denied,
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    sqlx::query("UPDATE ui_installations SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("restore installation for generation case");

    let stale_generation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         SELECT $1, installation_id, generation_no + 1, release_id, ui_key, ui_scope
         FROM ui_installation_generations WHERE id = $2",
    )
    .bind(stale_generation)
    .bind(fixture.generation)
    .execute(&worker)
    .await
    .expect("seed newer generation");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(stale_generation)
        .execute(&worker)
        .await
        .expect("activate newer generation");
    let stale_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(stale_denied, Err(UiBrowserHandoffError::PermissionDenied));

    let expiring_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp(),
                 statement_timestamp() + interval '1 second')",
    )
    .bind(expiring_parent)
    .bind(digest(220))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(221))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed short-lived parent");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let expired_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(expiring_parent),
            installation_id: installation,
            generation_id: UiInstallationGenerationId::from_uuid(stale_generation),
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(expired_denied, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke actor target authority");
    let target_revoked = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.global_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.global_generation),
            route: UiBrowserRoute::parse("schema-global").expect("global route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(target_revoked, Err(UiBrowserHandoffError::PermissionDenied));
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner') ON CONFLICT DO NOTHING",
    )
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore actor target authority");

    sqlx::query("UPDATE organization_members SET role = 'member' WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("demote actor for source permission case");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         SELECT project_id, $2 FROM ui_installations WHERE id = $1
         ON CONFLICT DO NOTHING",
    )
    .bind(fixture.other_installation)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("grant source project permission");
    let source_permission_handoff = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("source permission route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await
        .expect("project maintainer may use source release");
    let with_source_permission: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(&worker)
            .await
            .expect("count source permission handoff");
    assert_eq!(with_source_permission, baseline + 1);
    assert_eq!(source_permission_handoff.route.as_str(), "schema-ui-two");
    sqlx::query(
        "DELETE FROM project_maintainers
         WHERE project_id = (SELECT project_id FROM ui_installations WHERE id = $1)
           AND user_id = $2",
    )
    .bind(fixture.other_installation)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("revoke source project permission");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke source release permission");
    let source_permission_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("source permission route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(
        source_permission_denied,
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    sqlx::query("UPDATE organization_members SET role = 'owner' WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore actor owner role");

    let source_release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM ui_installation_generations WHERE id = $1")
            .bind(stale_generation)
            .fetch_one(&worker)
            .await
            .expect("read source release");
    sqlx::query(
        "UPDATE releases
         SET state = 'revoked', revoked_at = statement_timestamp()
         WHERE id = $1",
    )
    .bind(source_release)
    .execute(&worker)
    .await
    .expect("revoke source release");
    let source_revoked = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: UiInstallationGenerationId::from_uuid(stale_generation),
            route,
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(source_revoked, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE ui_installations SET lifecycle = 'removed', removed_at = statement_timestamp() WHERE id = $1")
        .bind(fixture.other_installation)
        .execute(&worker)
        .await
        .expect("remove alternate installation");
    let removed_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("alternate route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(removed_denied, Err(UiBrowserHandoffError::PermissionDenied));

    let final_count: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count final denied attempts");
    assert_eq!(final_count, baseline + 1);
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_exchange_is_atomic_generation_bound_and_parent_capped() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser exchange: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0089");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let store = PgUiBrowserSessionStore::new(worker.clone(), app);
    let actor = UserId::from_uuid(fixture.actor);
    let parent = BrowserSessionId::from_uuid(fixture.parent_session);
    let installation = UiInstallationId::from_uuid(fixture.installation);
    let generation = UiInstallationGenerationId::from_uuid(fixture.generation);
    let route = UiBrowserRoute::parse("schema-ui").expect("published route base");

    // A host resolved to another generation cannot exchange the locked handoff.
    let wrong_host_secret = UiBrowserHandoffSecret::from_bytes([41; 32]);
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        wrong_host_secret,
    )
    .await;
    let wrong_host = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes([41; 32]),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes([42; 32]),
        })
        .await;
    assert_eq!(wrong_host, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(&worker, UiBrowserHandoffSecret::from_bytes([41; 32])).await;

    let success_secret = UiBrowserHandoffSecret::from_bytes([43; 32]);
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        success_secret,
    )
    .await;
    let success = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes([43; 32]),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes([44; 32]),
        })
        .await
        .expect("valid handoff exchanges once");
    let child_row: (
        Vec<u8>,
        time::OffsetDateTime,
        time::OffsetDateTime,
        Uuid,
        Uuid,
    ) = sqlx::query_as(
        "SELECT session_digest, issued_at, expires_at, parent_session_id, generation_id
         FROM ui_browser_sessions WHERE id = $1",
    )
    .bind(success.context.session_id.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("read child safe metadata");
    assert_eq!(
        child_row.0,
        UiBrowserSessionSecret::from_bytes([44; 32])
            .digest()
            .as_bytes()
    );
    assert_eq!(child_row.3, fixture.parent_session);
    assert_eq!(child_row.4, fixture.generation);
    assert_eq!(child_row.2 - child_row.1, time::Duration::hours(12));
    let replay = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes([43; 32]),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes([45; 32]),
        })
        .await;
    assert_eq!(replay, Err(UiBrowserHandoffError::InvalidOrExpired));
    let replay_children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_browser_sessions WHERE handoff_id =
         (SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1)",
    )
    .bind(
        UiBrowserHandoffSecret::from_bytes([43; 32])
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .fetch_one(&worker)
    .await
    .expect("count replay children");
    assert_eq!(replay_children, 1);

    // Two named worker connections contend on one handoff row. An external
    // blocker makes both waits observable before release; after release only
    // one can insert a child and consume the handoff.
    let parallel_worker_a =
        parallel_role_pool(&database_url, "hephaestus_worker", "ui-browser-exchange-a").await;
    let parallel_worker_b =
        parallel_role_pool(&database_url, "hephaestus_worker", "ui-browser-exchange-b").await;
    let parallel_app_a = role_pool(&database_url, "hephaestus_app").await;
    let parallel_app_b = role_pool(&database_url, "hephaestus_app").await;
    let parallel_store_a = PgUiBrowserSessionStore::new(parallel_worker_a, parallel_app_a);
    let parallel_store_b = PgUiBrowserSessionStore::new(parallel_worker_b, parallel_app_b);
    let concurrent_secret = UiBrowserHandoffSecret::from_bytes([46; 32]);
    let concurrent_digest = concurrent_secret.digest().as_bytes().to_vec();
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        concurrent_secret,
    )
    .await;
    let mut exchange_blocker = bootstrap
        .begin()
        .await
        .expect("begin exchange lock barrier");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *exchange_blocker)
        .await
        .expect("read exchange lock barrier PID");
    sqlx::query("SELECT set_config('application_name', 'ui-browser-exchange-blocker', false)")
        .execute(&mut *exchange_blocker)
        .await
        .expect("name exchange lock barrier");
    println!("REAL_UI_BROWSER_EXCHANGE_BLOCKER=1 blocker_pid={blocker_pid}");
    sqlx::query("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1 FOR UPDATE")
        .bind(&concurrent_digest)
        .fetch_one(&mut *exchange_blocker)
        .await
        .expect("hold exchange handoff lock barrier");
    let first_task = tokio::spawn(async move {
        parallel_store_a
            .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
                request_id: RequestId::new(),
                handoff_secret: UiBrowserHandoffSecret::from_bytes([46; 32]),
                expected_generation_id: generation,
                child_secret: UiBrowserSessionSecret::from_bytes([47; 32]),
            })
            .await
    });
    let second_task = tokio::spawn(async move {
        parallel_store_b
            .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
                request_id: RequestId::new(),
                handoff_secret: UiBrowserHandoffSecret::from_bytes([46; 32]),
                expected_generation_id: generation,
                child_secret: UiBrowserSessionSecret::from_bytes([48; 32]),
            })
            .await
    });
    wait_for_named_exchange_lock_waiter(&bootstrap, "ui-browser-exchange-a", blocker_pid).await;
    wait_for_named_exchange_lock_waiter(&bootstrap, "ui-browser-exchange-b", blocker_pid).await;
    exchange_blocker
        .commit()
        .await
        .expect("release exchange lock barrier");
    let first = first_task.await.expect("first exchange task");
    let second = second_task.await.expect("second exchange task");
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        usize::from(first == Err(UiBrowserHandoffError::InvalidOrExpired))
            + usize::from(second == Err(UiBrowserHandoffError::InvalidOrExpired)),
        1
    );
    let concurrent_children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_browser_sessions WHERE handoff_id =
         (SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1)",
    )
    .bind(
        UiBrowserHandoffSecret::from_bytes([46; 32])
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .fetch_one(&worker)
    .await
    .expect("count concurrent children");
    assert_eq!(concurrent_children, 1);

    // A handoff outside its fixed window is rejected after current authority
    // checks and leaves no child.
    let expired_handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 statement_timestamp() - interval '61 seconds',
                 statement_timestamp() - interval '1 second')",
    )
    .bind(expired_handoff)
    .bind(
        UiBrowserHandoffSecret::from_bytes([49; 32])
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.organization)
    .bind("schema-ui")
    .execute(&worker)
    .await
    .expect("seed expired handoff");
    let expired = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes([49; 32]),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes([50; 32]),
        })
        .await;
    assert_eq!(expired, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(&worker, UiBrowserHandoffSecret::from_bytes([49; 32])).await;

    // Current generation and source publication are rechecked at exchange.
    let stale_secret = UiBrowserHandoffSecret::from_bytes([51; 32]);
    issue_handoff(
        &store,
        actor,
        parent,
        installation,
        generation,
        route,
        stale_secret,
    )
    .await;
    let stale_generation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         SELECT $1, installation_id, generation_no + 1, release_id, ui_key, ui_scope
         FROM ui_installation_generations WHERE id = $2",
    )
    .bind(stale_generation)
    .bind(fixture.generation)
    .execute(&worker)
    .await
    .expect("seed current generation replacement");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(stale_generation)
        .execute(&worker)
        .await
        .expect("activate current generation replacement");
    let stale = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes([51; 32]),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes([52; 32]),
        })
        .await;
    assert_eq!(stale, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(&worker, UiBrowserHandoffSecret::from_bytes([51; 32])).await;

    // Parent revocation/account state is checked before child creation.
    let revoked_secret = UiBrowserHandoffSecret::from_bytes([53; 32]);
    issue_handoff(
        &store,
        actor,
        parent,
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        revoked_secret,
    )
    .await;
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
         WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&worker)
    .await
    .expect("revoke parent session");
    let revoked = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes([53; 32]),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes([54; 32]),
        })
        .await;
    assert_eq!(revoked, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(&worker, UiBrowserHandoffSecret::from_bytes([53; 32])).await;

    let account_parent = Uuid::new_v4();
    insert_canonical_session(&worker, account_parent, fixture.actor, Uuid::new_v4(), 20).await;
    issue_handoff(
        &store,
        actor,
        BrowserSessionId::from_uuid(account_parent),
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        UiBrowserHandoffSecret::from_bytes([57; 32]),
    )
    .await;
    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend account");
    let account_suspended = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes([57; 32]),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes([58; 32]),
        })
        .await;
    assert_eq!(
        account_suspended,
        Err(UiBrowserHandoffError::InvalidOrExpired)
    );
    assert_exchange_denial_unchanged(&worker, UiBrowserHandoffSecret::from_bytes([57; 32])).await;
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore account");

    let source_parent = Uuid::new_v4();
    insert_canonical_session(&worker, source_parent, fixture.actor, Uuid::new_v4(), 20).await;
    issue_handoff(
        &store,
        actor,
        BrowserSessionId::from_uuid(source_parent),
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        UiBrowserHandoffSecret::from_bytes([55; 32]),
    )
    .await;
    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = statement_timestamp()
         WHERE id = (SELECT release_id FROM ui_installation_generations WHERE id = $1)",
    )
    .bind(fixture.other_generation)
    .execute(&worker)
    .await
    .expect("revoke source release");
    let source_revoked = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes([55; 32]),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes([56; 32]),
        })
        .await;
    assert_eq!(source_revoked, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(&worker, UiBrowserHandoffSecret::from_bytes([55; 32])).await;
}

async fn assert_exchange_denial_unchanged(pool: &PgPool, secret: UiBrowserHandoffSecret) {
    let digest = secret.digest().as_bytes().to_vec();
    let handoff_id: Uuid =
        sqlx::query_scalar("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1")
            .bind(&digest)
            .fetch_one(pool)
            .await
            .expect("find denied exchange handoff");
    let consumed_at: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(handoff_id)
            .fetch_one(pool)
            .await
            .expect("read denied exchange consumption");
    let children: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(handoff_id)
            .fetch_one(pool)
            .await
            .expect("count denied exchange children");
    assert!(
        consumed_at.is_none(),
        "denied exchange consumed its handoff"
    );
    assert_eq!(children, 0, "denied exchange created a child");
}

async fn issue_handoff(
    store: &PgUiBrowserSessionStore,
    actor: UserId,
    parent: BrowserSessionId,
    installation: UiInstallationId,
    generation: UiInstallationGenerationId,
    route: UiBrowserRoute,
    secret: UiBrowserHandoffSecret,
) {
    store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route,
            secret,
        })
        .await
        .expect("issue exchange fixture handoff");
}

async fn parallel_role_pool(database_url: &str, role: &str, application_name: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            let application_name = application_name.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                let application_name = application_name.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role)
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(application_name)
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
                        .bind(Uuid::nil().to_string())
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect parallel restricted PostgreSQL role")
}

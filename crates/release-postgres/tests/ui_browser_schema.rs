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
    for (ui_key, route_base) in [
        ("schema-ui", "schema-ui"),
        ("schema-ui-two", "schema-ui-two"),
    ] {
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, $2, 'project', $3, 'app', 'iframe', $4,
                     'index.html', 1, 'no_store', 'static')",
        )
        .bind(release)
        .bind(ui_key)
        .bind(ui_key)
        .bind(route_base)
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
                        .bind(role)
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

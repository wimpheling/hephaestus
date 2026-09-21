//! Real-role schema coverage for migration 0088 UI installations.
//!
//! Run with `HEPHAESTUS_POSTGRES_TEST_URL` and let the validation runner require
//! the `REAL_RELEASE_UI_INSTALLATION_SCHEMA=1` marker.

use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{borrow::Cow, env, time::Duration};
use tokio::{sync::oneshot, time::timeout};
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 88;

#[derive(Clone, Copy)]
struct Fixture {
    actor: Uuid,
    outsider: Uuid,
    project: Uuid,
    repository: Uuid,
    repository_two: Uuid,
    release: Uuid,
    second_release: Uuid,
    release_agent: Uuid,
    second_release_agent: Uuid,
    ui_key: &'static str,
    repository_ui_key: &'static str,
    gateway: Uuid,
    second_gateway: Uuid,
    gateway_revision: Uuid,
    second_gateway_revision: Uuid,
    installation: Uuid,
    generation: Uuid,
    repository_installation: Uuid,
    repository_generation: Uuid,
    repository_two_installation: Uuid,
    repository_two_generation: Uuid,
    binding_key: &'static str,
    command: [u8; 32],
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_installation_schema_enforces_fks_lifecycle_immutability_and_rls() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI installation schema: test URL is unset");
        return;
    };
    let bootstrap = admin_pool(&database_url).await;
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0088");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker exists");
    assert!(max_migration >= EXPECTED_MIGRATION);

    let worker = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let app = role_pool(&database_url, "hephaestus_app", Uuid::nil()).await;
    assert_role(&worker, "hephaestus_worker", false, true).await;
    assert_role(&app, "hephaestus_app", false, false).await;

    let fixture = seed_parent_rows(&worker).await;
    seed_installation_rows(&worker, &fixture).await;
    assert_worker_grants(&worker).await;
    assert_installation_identity_is_immutable(&worker, &fixture).await;
    assert_active_owner_key_is_unique(&worker, &fixture).await;
    assert_composite_fks(&worker, &fixture).await;
    assert_immutability_and_removed_terminal(&worker, &fixture).await;
    assert_application_rls(&database_url, &fixture).await;

    println!(
        "REAL_RELEASE_UI_INSTALLATION_SCHEMA=1 migration={max_migration} \
         app_role=hephaestus_app worker_role=hephaestus_worker \
         partial_unique=1 composite_fks=1 immutable=1 removed_terminal=1 app_rls=1"
    );
}

// This keeps the real publication-parent rows together so their FK ordering is reviewable.
#[allow(clippy::too_many_lines)]
async fn seed_parent_rows(pool: &PgPool) -> Fixture {
    let actor = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let repository_two = Uuid::new_v4();
    let build = Uuid::new_v4();
    let receive = Uuid::new_v4();
    let source_revision = Uuid::new_v4();
    let release = Uuid::new_v4();
    let family = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    let ui_key = "schema-ui";
    let repository_ui_key = "repo-ui";
    let gateway = Uuid::new_v4();
    let gateway_revision = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2), ($3, $4)")
        .bind(actor)
        .bind("UI schema owner")
        .bind(outsider)
        .bind("UI schema outsider")
        .execute(pool)
        .await
        .expect("seed users");
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(actor.to_string())
        .execute(pool)
        .await
        .expect("set fixture actor");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'UI schema org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(actor)
    .execute(pool)
    .await
    .expect("seed organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'ui-schema')")
        .bind(project)
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, 'ui-schema-repository'),
                ($3, $2, 'ui-schema-repository-two')",
    )
    .bind(repository)
    .bind(project)
    .bind(repository_two)
    .execute(pool)
    .await
    .expect("seed repository");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'ui-installation-schema', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository)
    .bind(actor)
    .execute(pool)
    .await
    .expect("seed source receive");
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
    .execute(pool)
    .await
    .expect("seed build request");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config, normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', $7, $8)",
    )
    .bind(source_revision)
    .bind(repository)
    .bind(receive)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .bind(vec![2_u8; 32])
    .bind(serde_json::json!({"version": 1, "uis": []}))
    .bind(vec![3_u8; 32])
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
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(source_revision)
    .execute(pool)
    .await
    .expect("link build to UI source capture");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         VALUES ($1, $2, 'ui-schema-v1', $3, 'refs/heads/main', $4, $5, '{}'::jsonb,
                 $6, $7, 'draft')",
    )
    .bind(release)
    .bind(repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(build)
    .bind(vec![1_u8; 32])
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(pool)
    .await
    .expect("seed published release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'ui-handler')",
    )
    .bind(family)
    .bind(repository)
    .execute(pool)
    .await
    .expect("seed release family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name, runtime_contract,
          runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'ui-handler', 'UI handler', '{}'::jsonb, $4, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(family)
    .bind(vec![4_u8; 32])
    .execute(pool)
    .await
    .expect("seed release agent");
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
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, $2, 'project', 'Schema UI', 'app', 'iframe', 'schema-ui',
                 'index.html', 1, 'no_store', 'static')",
    )
    .bind(release)
    .bind(ui_key)
    .execute(pool)
    .await
    .expect("seed published UI descriptor");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, $2, 'repository', 'Repository UI', 'app', 'iframe', 'repo-ui',
                 'index.html', 1, 'no_store', 'static')",
    )
    .bind(release)
    .bind(repository_ui_key)
    .execute(pool)
    .await
    .expect("seed repository UI descriptor");
    sqlx::query("UPDATE releases SET state = 'published', published_at = now() WHERE id = $1")
        .bind(release)
        .execute(pool)
        .await
        .expect("publish release after UI descriptors");
    sqlx::query(
        "INSERT INTO gateways
         (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'ui-gateway', 'enabled', $4)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(actor)
    .execute(pool)
    .await
    .expect("seed gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
          release_agent_key, handler_contract, exposure, parameters, normalized_hash,
          created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'ui-handler', 'http.v1',
                 'heph_authenticated', '{}'::jsonb, $7, $8)",
    )
    .bind(gateway_revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(release)
    .bind(release_agent)
    .bind(vec![5_u8; 32])
    .bind(actor)
    .execute(pool)
    .await
    .expect("seed gateway revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/ui', ARRAY['GET']::text[])",
    )
    .bind(Uuid::new_v4())
    .bind(gateway_revision)
    .bind(gateway)
    .bind(project)
    .execute(pool)
    .await
    .expect("seed gateway route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(gateway_revision)
        .execute(pool)
        .await
        .expect("activate gateway revision");

    let (second_release, second_release_agent, second_gateway, second_gateway_revision) =
        seed_second_published_fixture(pool, actor, project, repository).await;

    Fixture {
        actor,
        outsider,
        project,
        repository,
        repository_two,
        release,
        second_release,
        release_agent,
        second_release_agent,
        ui_key,
        repository_ui_key,
        gateway,
        second_gateway,
        gateway_revision,
        second_gateway_revision,
        installation: Uuid::new_v4(),
        generation: Uuid::new_v4(),
        repository_installation: Uuid::new_v4(),
        repository_generation: Uuid::new_v4(),
        repository_two_installation: Uuid::new_v4(),
        repository_two_generation: Uuid::new_v4(),
        binding_key: "api",
        command: [6_u8; 32],
    }
}

// Keep a second fully valid publication identity so composite-FK failures cannot
// be attributed to ordinary missing-parent foreign keys.
#[allow(clippy::too_many_lines)]
async fn seed_second_published_fixture(
    pool: &PgPool,
    actor: Uuid,
    project: Uuid,
    repository: Uuid,
) -> (Uuid, Uuid, Uuid, Uuid) {
    let receive = Uuid::new_v4();
    let build = Uuid::new_v4();
    let source_revision = Uuid::new_v4();
    let release = Uuid::new_v4();
    let family = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let gateway_revision = Uuid::new_v4();
    let commit = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'ui-installation-schema-two', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository)
    .bind(actor)
    .execute(pool)
    .await
    .expect("seed second source receive");
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
    .bind(vec![9_u8; 32])
    .bind(actor)
    .execute(pool)
    .await
    .expect("seed second build request");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config, normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', $7, $8)",
    )
    .bind(source_revision)
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .bind("dddddddddddddddddddddddddddddddddddddddd")
    .bind(vec![10_u8; 32])
    .bind(serde_json::json!({"version": 1, "uis": []}))
    .bind(vec![11_u8; 32])
    .execute(pool)
    .await
    .expect("seed second UI source capture");
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
    .expect("link second build to UI source capture");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         VALUES ($1, $2, 'ui-schema-v2', $3, 'refs/heads/main', $4, $5, '{}'::jsonb,
                 $6, $7, 'draft')",
    )
    .bind(release)
    .bind(repository)
    .bind(commit)
    .bind(build)
    .bind(vec![9_u8; 32])
    .bind(vec![12_u8; 32])
    .bind(vec![13_u8; 32])
    .execute(pool)
    .await
    .expect("seed second draft release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'ui-handler-two')",
    )
    .bind(family)
    .bind(repository)
    .execute(pool)
    .await
    .expect("seed second agent family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name, runtime_contract,
          runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'ui-handler-two', 'UI handler two', '{}'::jsonb, $4, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(family)
    .bind(vec![14_u8; 32])
    .execute(pool)
    .await
    .expect("seed second release agent");
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
    .expect("seed second release UI snapshot");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'schema-ui-two', 'project', 'Schema UI Two', 'app', 'iframe',
                 'schema-ui-two', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(release)
    .execute(pool)
    .await
    .expect("seed second UI descriptor");
    for (ui_key, route_base) in [
        ("schema-global-two", "schema-global-two"),
        ("schema-global-cross", "schema-global-cross"),
    ] {
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, $2, 'global', 'Global Schema UI', 'app', 'iframe', $3,
                     'index.html', 1, 'no_store', 'static')",
        )
        .bind(release)
        .bind(ui_key)
        .bind(route_base)
        .execute(pool)
        .await
        .expect("seed second global UI descriptor");
    }
    sqlx::query("UPDATE releases SET state = 'published', published_at = now() WHERE id = $1")
        .bind(release)
        .execute(pool)
        .await
        .expect("publish second release");
    sqlx::query(
        "INSERT INTO gateways
         (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'ui-gateway-two', 'enabled', $4)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(actor)
    .execute(pool)
    .await
    .expect("seed second gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
          release_agent_key, handler_contract, exposure, parameters, normalized_hash,
          created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'ui-handler-two', 'http.v1',
                 'heph_authenticated', '{}'::jsonb, $7, $8)",
    )
    .bind(gateway_revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(release)
    .bind(release_agent)
    .bind(vec![15_u8; 32])
    .bind(actor)
    .execute(pool)
    .await
    .expect("seed second gateway revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/ui-two', ARRAY['GET']::text[])",
    )
    .bind(Uuid::new_v4())
    .bind(gateway_revision)
    .bind(gateway)
    .bind(project)
    .execute(pool)
    .await
    .expect("seed second gateway route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(gateway_revision)
        .execute(pool)
        .await
        .expect("activate second gateway revision");
    (release, release_agent, gateway, gateway_revision)
}

async fn seed_installation_rows(pool: &PgPool, fixture: &Fixture) {
    let mut tx = pool.begin().await.expect("begin installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'project', $4, 'enabled', $5, $6)",
    )
    .bind(fixture.installation)
    .bind(fixture.project)
    .bind(Option::<Uuid>::None)
    .bind(fixture.ui_key)
    .bind(fixture.generation)
    .bind(fixture.actor)
    .execute(&mut *tx)
    .await
    .expect("seed installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'project')",
    )
    .bind(fixture.generation)
    .bind(fixture.installation)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed installation generation");
    sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key, release_id,
          ui_key, gateway_id, gateway_revision_id, release_agent_id, gateway_name,
          method, route, exposure)
         VALUES ($1, $2, 'api', $3, $4, $5, $6, $7, $8, 'ui-gateway',
                 'GET', '/ui', 'heph_authenticated')",
    )
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.binding_key)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .bind(fixture.gateway)
    .bind(fixture.gateway_revision)
    .bind(fixture.release_agent)
    .execute(&mut *tx)
    .await
    .expect("seed installation binding");
    sqlx::query(
        "INSERT INTO ui_installation_commands
         (command_key, caller_idempotency_key, operation, installation_id, actor_id,
          request_id, input_hash, result_generation_id, result_lifecycle)
         VALUES ($1, 'schema-install', 'install', $2, $3, $4, $5, $6, 'enabled')",
    )
    .bind(fixture.command.as_slice())
    .bind(fixture.installation)
    .bind(fixture.actor)
    .bind(Uuid::new_v4())
    .bind(vec![7_u8; 32])
    .bind(fixture.generation)
    .execute(&mut *tx)
    .await
    .expect("seed installation command");
    tx.commit().await.expect("commit installation seed");
    seed_repository_installation(
        pool,
        fixture,
        fixture.repository_installation,
        fixture.repository_generation,
        fixture.repository,
    )
    .await;
    seed_repository_installation(
        pool,
        fixture,
        fixture.repository_two_installation,
        fixture.repository_two_generation,
        fixture.repository_two,
    )
    .await;
}

async fn seed_repository_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
    repository: Uuid,
) {
    let mut tx = pool
        .begin()
        .await
        .expect("begin repository installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'repository', $4, 'enabled', $5, $6)",
    )
    .bind(installation)
    .bind(fixture.project)
    .bind(repository)
    .bind(fixture.repository_ui_key)
    .bind(generation)
    .bind(fixture.actor)
    .execute(&mut *tx)
    .await
    .expect("seed repository installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'repository')",
    )
    .bind(generation)
    .bind(installation)
    .bind(fixture.release)
    .bind(fixture.repository_ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed repository generation");
    tx.commit()
        .await
        .expect("commit repository installation seed");
}

async fn assert_active_owner_key_is_unique(pool: &PgPool, fixture: &Fixture) {
    let duplicate_project = sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, 'project', $3, 'enabled', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.project)
    .bind(fixture.ui_key)
    .bind(fixture.generation)
    .bind(fixture.actor)
    .execute(pool)
    .await;
    assert_sqlstate(duplicate_project, "23505");

    let duplicate_repository = sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, 'repository', $4, 'enabled', $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.project)
    .bind(fixture.repository)
    .bind(fixture.repository_ui_key)
    .bind(fixture.repository_generation)
    .bind(fixture.actor)
    .execute(pool)
    .await;
    assert_sqlstate(duplicate_repository, "23505");

    sqlx::query(
        "UPDATE ui_installations
         SET lifecycle = 'removed', removed_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(fixture.installation)
    .execute(pool)
    .await
    .expect("remove original project installation");
    let replacement_installation = Uuid::new_v4();
    let replacement_generation = Uuid::new_v4();
    seed_project_installation(
        pool,
        fixture,
        replacement_installation,
        replacement_generation,
    )
    .await;

    sqlx::query(
        "UPDATE ui_installations
         SET lifecycle = 'removed', removed_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(fixture.repository_installation)
    .execute(pool)
    .await
    .expect("remove original repository installation");
    let repository_replacement_installation = Uuid::new_v4();
    let repository_replacement_generation = Uuid::new_v4();
    seed_repository_installation(
        pool,
        fixture,
        repository_replacement_installation,
        repository_replacement_generation,
        fixture.repository,
    )
    .await;
}

async fn seed_project_installation(
    pool: &PgPool,
    fixture: &Fixture,
    installation: Uuid,
    generation: Uuid,
) {
    let mut tx = pool.begin().await.expect("begin project replacement");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, NULL, 'project', $3, 'enabled', $4, $5)",
    )
    .bind(installation)
    .bind(fixture.project)
    .bind(fixture.ui_key)
    .bind(generation)
    .bind(fixture.actor)
    .execute(&mut *tx)
    .await
    .expect("reuse removed project owner key");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'project')",
    )
    .bind(generation)
    .bind(installation)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .execute(&mut *tx)
    .await
    .expect("replacement project generation");
    tx.commit().await.expect("commit project replacement");
}

async fn assert_worker_grants(pool: &PgPool) {
    for table in [
        "ui_installations",
        "ui_installation_generations",
        "ui_installation_bindings",
        "ui_installation_commands",
    ] {
        let can_insert: bool =
            sqlx::query_scalar("SELECT has_table_privilege(current_user, $1, 'INSERT')")
                .bind(table)
                .fetch_one(pool)
                .await
                .expect("read worker INSERT privilege");
        assert!(can_insert, "worker can append to {table}");
        let can_update: bool =
            sqlx::query_scalar("SELECT has_table_privilege(current_user, $1, 'UPDATE')")
                .bind(table)
                .fetch_one(pool)
                .await
                .expect("read worker UPDATE privilege");
        let can_delete: bool =
            sqlx::query_scalar("SELECT has_table_privilege(current_user, $1, 'DELETE')")
                .bind(table)
                .fetch_one(pool)
                .await
                .expect("read worker DELETE privilege");
        assert_eq!(
            can_update,
            table == "ui_installations",
            "worker UPDATE privilege for {table}"
        );
        assert!(!can_delete, "worker cannot delete {table}");
    }
}

async fn assert_installation_identity_is_immutable(pool: &PgPool, fixture: &Fixture) {
    let id_update = sqlx::query("UPDATE ui_installations SET id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(Uuid::new_v4())
        .execute(pool)
        .await;
    assert_sqlstate(id_update, "23000");

    let creator_update = sqlx::query("UPDATE ui_installations SET created_by = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(Uuid::new_v4())
        .execute(pool)
        .await;
    assert_sqlstate(creator_update, "23000");

    let created_at_update = sqlx::query(
        "UPDATE ui_installations
         SET created_at = created_at + interval '1 second'
         WHERE id = $1",
    )
    .bind(fixture.installation)
    .execute(pool)
    .await;
    assert_sqlstate(created_at_update, "23000");
}

async fn assert_composite_fks(pool: &PgPool, fixture: &Fixture) {
    let bad_generation_release_ui = sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 99, $3, $4, 'project')",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.installation)
    .bind(fixture.second_release)
    .bind(fixture.ui_key)
    .execute(pool)
    .await;
    assert_sqlstate(bad_generation_release_ui, "23503");

    let bad_scope_key = sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 99, $3, $4, 'repository')",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.repository_two_installation)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .execute(pool)
    .await;
    assert_sqlstate(bad_scope_key, "23503");

    let bad_current_generation =
        sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
            .bind(fixture.repository_two_installation)
            .bind(fixture.generation)
            .execute(pool)
            .await;
    assert_sqlstate(bad_current_generation, "23503");

    // Every referenced release/UI/gateway/agent exists, but the generation's
    // immutable release/UI tuple does not match the supplied binding.
    let bad_binding_release_ui = sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key, release_id,
          ui_key, gateway_id, gateway_revision_id, release_agent_id, gateway_name,
          method, route, exposure)
         VALUES ($1, $2, 'api', 'second-release-ui', $3, 'schema-ui-two', $4, $5, $6,
                 'ui-gateway-two', 'GET', '/ui-two', 'heph_authenticated')",
    )
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.second_release)
    .bind(fixture.second_gateway)
    .bind(fixture.second_gateway_revision)
    .bind(fixture.second_release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(bad_binding_release_ui, "23503");

    // The release agent exists, but it does not match the exact gateway
    // revision identity tuple.
    let bad_binding_agent = sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key, release_id,
          ui_key, gateway_id, gateway_revision_id, release_agent_id, gateway_name,
          method, route, exposure)
         VALUES ($1, $2, 'api', 'wrong-agent', $3, $4, $5, $6, $7,
                 'ui-gateway', 'GET', '/ui', 'heph_authenticated')",
    )
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .bind(fixture.gateway)
    .bind(fixture.gateway_revision)
    .bind(fixture.second_release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(bad_binding_agent, "23503");
}

async fn assert_immutability_and_removed_terminal(pool: &PgPool, fixture: &Fixture) {
    let generation_update =
        sqlx::query("UPDATE ui_installation_generations SET generation_no = 2 WHERE id = $1")
            .bind(fixture.generation)
            .execute(pool)
            .await;
    assert_sqlstate(generation_update, "42501");

    let binding_delete = sqlx::query(
        "DELETE FROM ui_installation_bindings
         WHERE generation_id = $1 AND binding_kind = 'api' AND binding_key = $2",
    )
    .bind(fixture.generation)
    .bind(fixture.binding_key)
    .execute(pool)
    .await;
    assert_sqlstate(binding_delete, "42501");

    let command_update =
        sqlx::query("UPDATE ui_installation_commands SET input_hash = $2 WHERE command_key = $1")
            .bind(fixture.command.as_slice())
            .bind(vec![8_u8; 32])
            .execute(pool)
            .await;
    assert_sqlstate(command_update, "42501");

    let terminal_update =
        sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
            .bind(fixture.installation)
            .execute(pool)
            .await;
    assert_sqlstate(terminal_update, "23000");
}

async fn assert_application_rls(database_url: &str, fixture: &Fixture) {
    let owner = role_pool(database_url, "hephaestus_app", fixture.actor).await;
    let forced_rls: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM pg_class
         WHERE relname IN (
             'ui_installations', 'ui_installation_generations',
             'ui_installation_bindings', 'ui_installation_commands'
         )
         AND relrowsecurity AND relforcerowsecurity",
    )
    .fetch_one(&owner)
    .await
    .expect("read UI installation RLS flags");
    assert_eq!(forced_rls, 4, "all UI installation tables use FORCE RLS");

    let visible: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ui_installations WHERE id = $1),
            (SELECT count(*) FROM ui_installation_generations WHERE installation_id = $1),
            (SELECT count(*) FROM ui_installation_bindings WHERE installation_id = $1),
            (SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1)",
    )
    .bind(fixture.installation)
    .fetch_one(&owner)
    .await
    .expect("authorized application read");
    assert_eq!(
        visible,
        (1, 1, 1, 1),
        "authorized owner can inspect retained history"
    );

    let outsider = role_pool(database_url, "hephaestus_app", fixture.outsider).await;
    let hidden: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&outsider)
        .await
        .expect("outsider installation read");
    assert_eq!(hidden, 0);
    let hidden_children: (i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ui_installation_generations),
            (SELECT count(*) FROM ui_installation_bindings),
            (SELECT count(*) FROM ui_installation_commands)",
    )
    .fetch_one(&outsider)
    .await
    .expect("outsider child installation reads");
    assert_eq!(hidden_children, (0, 0, 0));

    let denied_insert = sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, scope, ui_key, lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, 'project', 'app-write', 'enabled', $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.project)
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .execute(&owner)
    .await;
    assert_sqlstate(denied_insert, "42501");

    let denied_update = sqlx::query("UPDATE ui_installations SET updated_at = now() WHERE id = $1")
        .bind(fixture.installation)
        .execute(&owner)
        .await;
    assert_sqlstate(denied_update, "42501");
}

async fn admin_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
        .expect("connect migration bootstrap pool")
}

async fn role_pool(database_url: &str, role: &str, actor: Uuid) -> PgPool {
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
                        .bind(actor.to_string())
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

async fn assert_role(pool: &PgPool, expected: &str, superuser: bool, bypassrls: bool) {
    let row: (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .expect("read PostgreSQL role identity");
    assert_eq!(row, (expected.to_owned(), superuser, bypassrls));
}

fn assert_sqlstate<T>(result: Result<T, sqlx::Error>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("expected PostgreSQL error {expected}"),
        Err(error) => error,
    };
    let code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(Cow::into_owned);
    assert_eq!(code.as_deref(), Some(expected));
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn global_ui_installation_scope_tenant_invariant_and_dual_membership_rls() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping global UI installation schema: test URL is unset");
        return;
    };
    let bootstrap = admin_pool(&database_url).await;
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0088");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker exists");
    assert!(max_migration >= 88);

    let worker = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let member_a = Uuid::new_v4();
    let member_b = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization_b = Uuid::new_v4();
    let project_b = Uuid::new_v4();
    let repository_b = Uuid::new_v4();
    let unreferenced_project = Uuid::new_v4();
    let unreferenced_repository = Uuid::new_v4();
    let source_fixture = seed_parent_rows(&worker).await;
    let actor = source_fixture.actor;
    let organization_a: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(source_fixture.project)
            .fetch_one(&worker)
            .await
            .expect("read source organization");
    sqlx::query(
        "INSERT INTO users (id, display_name) VALUES
         ($1, 'global member A'), ($2, 'global member B'),
         ($3, 'global outsider')",
    )
    .bind(member_a)
    .bind(member_b)
    .bind(outsider)
    .execute(&worker)
    .await
    .expect("seed global users");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(organization_a)
    .bind(member_a)
    .execute(&worker)
    .await
    .expect("seed organization A member");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Global UI org B')")
        .bind(organization_b)
        .execute(&worker)
        .await
        .expect("seed organization B");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member'), ($1, $3, 'member')",
    )
    .bind(organization_b)
    .bind(actor)
    .bind(member_b)
    .execute(&worker)
    .await
    .expect("seed organization B members");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'global-b')")
        .bind(project_b)
        .bind(organization_b)
        .execute(&worker)
        .await
        .expect("seed organization B project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, 'global-b-repository')",
    )
    .bind(repository_b)
    .bind(project_b)
    .execute(&worker)
    .await
    .expect("seed organization B repository");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'unreferenced')")
        .bind(unreferenced_project)
        .bind(organization_a)
        .execute(&worker)
        .await
        .expect("seed unreferenced project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, 'unreferenced-repository')",
    )
    .bind(unreferenced_repository)
    .bind(unreferenced_project)
    .execute(&worker)
    .await
    .expect("seed unreferenced repository");
    let (release_b, _, _, _) =
        seed_second_published_fixture(&worker, actor, project_b, repository_b).await;

    let installation_a = Uuid::new_v4();
    let generation_a = Uuid::new_v4();
    seed_global_installation(
        &worker,
        organization_a,
        installation_a,
        generation_a,
        source_fixture.second_release,
        "schema-global-two",
    )
    .await;
    let installation_b = Uuid::new_v4();
    let generation_b = Uuid::new_v4();
    seed_global_installation(
        &worker,
        organization_b,
        installation_b,
        generation_b,
        release_b,
        "schema-global-two",
    )
    .await;

    let duplicate = sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(organization_a)
    .bind("schema-global-two")
    .bind(Uuid::new_v4())
    .bind(actor)
    .execute(&worker)
    .await;
    assert_sqlstate(duplicate, "23505");

    let cross_owner_installation = Uuid::new_v4();
    let cross_owner_generation = Uuid::new_v4();
    let mut cross_owner = worker.begin().await.expect("begin cross-owner seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', 'schema-global-cross',
                 'enabled', $3, $4)",
    )
    .bind(cross_owner_installation)
    .bind(organization_b)
    .bind(cross_owner_generation)
    .bind(actor)
    .execute(&mut *cross_owner)
    .await
    .expect("seed cross-owner installation identity");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, 'schema-global-cross', 'global')",
    )
    .bind(cross_owner_generation)
    .bind(cross_owner_installation)
    .bind(source_fixture.second_release)
    .execute(&mut *cross_owner)
    .await
    .expect("seed cross-owner generation before deferred check");
    let cross_owner_result = cross_owner.commit().await;
    assert_sqlstate(cross_owner_result, "23000");

    let moved_unreferenced_project =
        sqlx::query("UPDATE projects SET organization_id = $2 WHERE id = $1")
            .bind(unreferenced_project)
            .bind(organization_b)
            .execute(&worker)
            .await;
    moved_unreferenced_project.expect("unreferenced project may change organization");
    let moved_unreferenced_repository =
        sqlx::query("UPDATE repositories SET project_id = $2 WHERE id = $1")
            .bind(unreferenced_repository)
            .bind(project_b)
            .execute(&worker)
            .await;
    moved_unreferenced_repository.expect("unreferenced repository may change project");

    let moved_referenced_project =
        sqlx::query("UPDATE projects SET organization_id = $2 WHERE id = $1")
            .bind(source_fixture.project)
            .bind(organization_b)
            .execute(&worker)
            .await;
    assert_sqlstate(moved_referenced_project, "23000");
    let moved_referenced_repository =
        sqlx::query("UPDATE repositories SET project_id = $2 WHERE id = $1")
            .bind(source_fixture.repository)
            .bind(project_b)
            .execute(&worker)
            .await;
    assert_sqlstate(moved_referenced_repository, "23000");
    let moved_referenced_release =
        sqlx::query("UPDATE releases SET repository_id = $2 WHERE id = $1")
            .bind(source_fixture.second_release)
            .bind(repository_b)
            .execute(&worker)
            .await;
    assert_sqlstate(moved_referenced_release, "23000");

    for (scope, organization_id, project_id, repository_id) in [
        (
            "global",
            Some(organization_a),
            Some(source_fixture.project),
            None,
        ),
        (
            "project",
            Some(organization_a),
            Some(source_fixture.project),
            None,
        ),
        ("repository", None, None, Some(source_fixture.repository)),
    ] {
        let invalid = sqlx::query(
            "INSERT INTO ui_installations
             (id, organization_id, project_id, repository_id, scope, ui_key,
              lifecycle, current_generation_id, created_by)
             VALUES ($1, $2, $3, $4, $5, 'invalid-owner-shape', 'enabled', $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(organization_id)
        .bind(project_id)
        .bind(repository_id)
        .bind(scope)
        .bind(Uuid::new_v4())
        .bind(actor)
        .execute(&worker)
        .await;
        assert_sqlstate(invalid, "23514");
    }

    let actor_app = role_pool(&database_url, "hephaestus_app", actor).await;
    let actor_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&actor_app)
        .await
        .expect("dual-member actor global read");
    assert_eq!(actor_visible, 2);
    let actor_a_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_installations WHERE organization_id = $1")
            .bind(organization_a)
            .fetch_one(&actor_app)
            .await
            .expect("explicit organization A filter");
    let actor_b_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_installations WHERE organization_id = $1")
            .bind(organization_b)
            .fetch_one(&actor_app)
            .await
            .expect("explicit organization B filter");
    assert_eq!((actor_a_visible, actor_b_visible), (1, 1));

    let member_a_app = role_pool(&database_url, "hephaestus_app", member_a).await;
    let member_a_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&member_a_app)
        .await
        .expect("organization A member global read");
    assert_eq!(member_a_visible, 1);
    let member_b_app = role_pool(&database_url, "hephaestus_app", member_b).await;
    let member_b_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&member_b_app)
        .await
        .expect("organization B member global read");
    assert_eq!(member_b_visible, 1);
    let outsider_app = role_pool(&database_url, "hephaestus_app", outsider).await;
    let outsider_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&outsider_app)
        .await
        .expect("outsider global read");
    assert_eq!(outsider_visible, 0);
    println!(
        "REAL_GLOBAL_UI_INSTALLATION_SCHEMA=1 migration={max_migration} \
         same_key_two_orgs=1 cross_owner_rejected=1 dual_member_explicit_filter=1 \
         owner_shape=1 organization_rls=1"
    );
}

#[tokio::test]
#[serial]
// Keep the two-connection barrier sequence together so its lock ordering is
// auditable in the schema proof.
#[allow(clippy::too_many_lines)]
async fn generation_insert_barrier_rejects_concurrent_source_project_move() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping generation/source-parent barrier: test URL is unset");
        return;
    };
    let bootstrap = admin_pool(&database_url).await;
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0088");

    let generation_pool = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let parent_pool = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let fixture = seed_parent_rows(&generation_pool).await;
    let source_organization: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(fixture.project)
            .fetch_one(&generation_pool)
            .await
            .expect("read source organization");
    let moved_organization = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Barrier destination')")
        .bind(moved_organization)
        .execute(&generation_pool)
        .await
        .expect("seed destination organization");

    let installation_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let mut generation_transaction = generation_pool
        .begin()
        .await
        .expect("begin generation transaction");
    let generation_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *generation_transaction)
        .await
        .expect("read generation transaction PID");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', 'schema-global-two',
                 'enabled', $3, $4)",
    )
    .bind(installation_id)
    .bind(source_organization)
    .bind(generation_id)
    .bind(fixture.actor)
    .execute(&mut *generation_transaction)
    .await
    .expect("insert barrier installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, 'schema-global-two', 'global')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(fixture.second_release)
    .execute(&mut *generation_transaction)
    .await
    .expect("insert deferred barrier generation");

    let mut parent_transaction = parent_pool.begin().await.expect("begin parent move");
    let parent_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *parent_transaction)
        .await
        .expect("read parent transaction PID");
    sqlx::query("UPDATE projects SET organization_id = $2 WHERE id = $1")
        .bind(fixture.project)
        .bind(moved_organization)
        .execute(&mut *parent_transaction)
        .await
        .expect("uncommitted source move passes before generation is visible");

    let (commit_started_sender, commit_started_receiver) = oneshot::channel();
    let generation_commit = tokio::spawn(async move {
        commit_started_sender
            .send(())
            .expect("generation commit receiver");
        generation_transaction.commit().await
    });
    commit_started_receiver
        .await
        .expect("generation commit started");
    wait_until_blocked(&bootstrap, generation_pid, parent_pid).await;
    parent_transaction
        .commit()
        .await
        .expect("source move commits before generation validation");
    let generation_result = generation_commit
        .await
        .expect("generation commit task join");
    assert_sqlstate(generation_result, "23000");

    let remaining_rows: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ui_installations WHERE id = $1),
            (SELECT count(*) FROM ui_installation_generations WHERE id = $2)",
    )
    .bind(installation_id)
    .bind(generation_id)
    .fetch_one(&bootstrap)
    .await
    .expect("inspect rolled-back generation");
    assert_eq!(remaining_rows, (0, 0));
}

#[tokio::test]
#[serial]
// Keep the two-connection source-release lock barrier beside the retained
// generation parent guard; the source-project barrier covers re-read failure.
#[allow(clippy::too_many_lines)]
async fn generation_barrier_locks_draft_source_release_before_mutation() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping source-release barrier: test URL is unset");
        return;
    };
    let bootstrap = admin_pool(&database_url).await;
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0088");

    let generation_pool = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let parent_pool = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let fixture = seed_parent_rows(&generation_pool).await;
    let source_organization: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(fixture.project)
            .fetch_one(&generation_pool)
            .await
            .expect("read source organization");
    let destination_organization = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Release barrier destination')")
        .bind(destination_organization)
        .execute(&generation_pool)
        .await
        .expect("seed release destination organization");

    let draft_release = Uuid::new_v4();
    let draft_ui_key = "schema-release-barrier";
    seed_draft_global_release(
        &generation_pool,
        fixture.release,
        draft_release,
        draft_ui_key,
    )
    .await;
    let installation_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let mut generation_transaction = generation_pool
        .begin()
        .await
        .expect("begin source-release generation transaction");
    let generation_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *generation_transaction)
        .await
        .expect("read source-release generation PID");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4, $5)",
    )
    .bind(installation_id)
    .bind(source_organization)
    .bind(draft_ui_key)
    .bind(generation_id)
    .bind(fixture.actor)
    .execute(&mut *generation_transaction)
    .await
    .expect("insert source-release barrier installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'global')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(draft_release)
    .bind(draft_ui_key)
    .execute(&mut *generation_transaction)
    .await
    .expect("insert source-release barrier generation");

    let mut release_move_transaction = parent_pool
        .begin()
        .await
        .expect("begin draft source-release move");
    let release_move_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *release_move_transaction)
        .await
        .expect("read source-release move PID");
    sqlx::query("UPDATE releases SET version = $2 WHERE id = $1")
        .bind(draft_release)
        .bind(format!("ui-barrier-moved-{draft_release}"))
        .execute(&mut *release_move_transaction)
        .await
        .expect("draft source-release mutation is initially unreferenced");

    let (commit_started_sender, commit_started_receiver) = oneshot::channel();
    let generation_commit = tokio::spawn(async move {
        commit_started_sender
            .send(())
            .expect("source-release commit receiver");
        generation_transaction.commit().await
    });
    commit_started_receiver
        .await
        .expect("source-release generation commit started");
    wait_until_blocked(&bootstrap, generation_pid, release_move_pid).await;
    release_move_transaction
        .commit()
        .await
        .expect("draft source-release mutation commits before validation");
    let generation_result = generation_commit
        .await
        .expect("source-release generation commit task join");
    generation_result.expect("generation validation succeeds after source-release lock");

    let remaining_rows: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ui_installations WHERE id = $1),
            (SELECT count(*) FROM ui_installation_generations WHERE id = $2)",
    )
    .bind(installation_id)
    .bind(generation_id)
    .fetch_one(&bootstrap)
    .await
    .expect("inspect source-release barrier generation");
    assert_eq!(remaining_rows, (1, 1));

    let generation_first_key = "schema-release-retained";
    let generation_first_release = Uuid::new_v4();
    seed_draft_global_release(
        &generation_pool,
        fixture.release,
        generation_first_release,
        generation_first_key,
    )
    .await;
    let generation_first_installation = Uuid::new_v4();
    let generation_first_generation = Uuid::new_v4();
    seed_global_installation(
        &generation_pool,
        source_organization,
        generation_first_installation,
        generation_first_generation,
        generation_first_release,
        generation_first_key,
    )
    .await;
    let rejected_parent_move =
        sqlx::query("UPDATE projects SET organization_id = $2 WHERE id = $1")
            .bind(fixture.project)
            .bind(destination_organization)
            .execute(&parent_pool)
            .await;
    assert_sqlstate(rejected_parent_move, "23000");
}

async fn seed_draft_global_release(
    pool: &PgPool,
    template_release: Uuid,
    release_id: Uuid,
    ui_key: &str,
) {
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         SELECT $1, repository_id, $3, source_commit, source_ref, build_request_id,
                build_definition_hash, configuration, configuration_hash, manifest_hash,
                'draft'
         FROM releases
         WHERE id = $2",
    )
    .bind(release_id)
    .bind(template_release)
    .bind(format!("ui-barrier-{release_id}"))
    .execute(pool)
    .await
    .expect("seed draft source release");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         SELECT $1, build_request_id, source_manifest_revision_id
         FROM release_ui_source_snapshots
         WHERE release_id = $2",
    )
    .bind(release_id)
    .bind(template_release)
    .execute(pool)
    .await
    .expect("seed draft source snapshot");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, $2, 'global', 'Release Barrier UI', 'app', 'iframe', $2,
                 'index.html', 1, 'no_store', 'static')",
    )
    .bind(release_id)
    .bind(ui_key)
    .execute(pool)
    .await
    .expect("seed draft global descriptor");
}

async fn seed_global_installation(
    pool: &PgPool,
    organization_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
) {
    let mut tx = pool.begin().await.expect("begin global installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4,
                 (SELECT user_id FROM organization_members
                  WHERE organization_id = $2 ORDER BY user_id LIMIT 1))",
    )
    .bind(installation_id)
    .bind(organization_id)
    .bind(ui_key)
    .bind(generation_id)
    .execute(&mut *tx)
    .await
    .expect("seed global installation");
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
    .expect("seed global generation");
    tx.commit().await.expect("commit global installation seed");
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
    .expect("generation transaction reached the source-parent lock barrier");
}

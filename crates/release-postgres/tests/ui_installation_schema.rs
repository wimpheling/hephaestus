//! Real-role schema coverage for migration 0087 UI installations.
//!
//! Run with `HEPHAESTUS_POSTGRES_TEST_URL` and let the validation runner require
//! the `REAL_RELEASE_UI_INSTALLATION_SCHEMA=1` marker.

use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{borrow::Cow, env, time::Duration};
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 87;

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
        .expect("apply migrations through 0087");
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

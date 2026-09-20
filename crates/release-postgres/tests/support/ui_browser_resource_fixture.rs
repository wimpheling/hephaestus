//! Shared real-PostgreSQL fixture for focused UI resource tests.

use release_domain::ui_browser::UiBrowserSessionSecret;
use sqlx::PgPool;
use uuid::Uuid;

// The shared fixture also feeds the gateway and navigation matrices; each
// resource test intentionally consumes only the identities relevant to it.
#[allow(dead_code)]
pub struct Fixture {
    pub actor: Uuid,
    pub outsider: Uuid,
    pub organization: Uuid,
    pub project: Uuid,
    pub source_project: Uuid,
    pub release: Uuid,
    pub release_agent: Uuid,
    pub other_organization: Uuid,
    pub parent_session: Uuid,
    pub outsider_parent_session: Uuid,
    pub installation: Uuid,
    pub other_installation: Uuid,
    pub generation: Uuid,
    pub other_generation: Uuid,
    pub global_installation: Uuid,
    pub global_generation: Uuid,
    pub repository_installation: Uuid,
    pub repository_generation: Uuid,
    pub managed_installation: Uuid,
    pub managed_generation: Uuid,
    pub managed_gateway: Uuid,
    pub managed_revision: Uuid,
    pub route: &'static str,
}

pub async fn seed_fixture_reusing_installation_helpers(worker: &PgPool) -> Fixture {
    seed_fixture_reusing_installation_helpers_with_publication(worker, true).await
}

pub async fn seed_fixture_reusing_installation_helpers_draft(worker: &PgPool) -> Fixture {
    seed_fixture_reusing_installation_helpers_with_publication(worker, false).await
}

/// Derive a stable test secret from the fixture identity while keeping it
/// unique across fixture runs that share a real `PostgreSQL` database.
pub fn fixture_session_secret(actor: Uuid, seed: u8) -> [u8; 32] {
    let mut secret = [seed; 32];
    secret[..16].copy_from_slice(actor.as_bytes());
    secret[16..].fill(seed);
    secret
}

// This integration fixture deliberately seeds the full release/gateway graph
// so the application-role projection is exercised against canonical rows.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
async fn seed_fixture_reusing_installation_helpers_with_publication(
    worker: &PgPool,
    publish_release: bool,
) -> Fixture {
    let actor = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let other_organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let source_project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let receive = Uuid::new_v4();
    let build = Uuid::new_v4();
    let source_revision = Uuid::new_v4();
    let release = Uuid::new_v4();
    let artifact = Uuid::new_v4();
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
    let managed_installation = Uuid::new_v4();
    let managed_generation = Uuid::new_v4();
    let managed_gateway = Uuid::new_v4();
    let managed_revision = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    let agent_family = Uuid::new_v4();
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
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $2, 'ui-browser-target'), ($3, $2, 'ui-browser-source')",
    )
    .bind(project)
    .bind(organization)
    .bind(source_project)
    .execute(worker)
    .await
    .expect("seed project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'ui-browser-repository')",
    )
    .bind(repository)
    .bind(source_project)
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
    .execute(worker)
    .await
    .expect("seed static UI artifact");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'browser-service')",
    )
    .bind(agent_family)
    .bind(repository)
    .execute(worker)
    .await
    .expect("seed browser service family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'browser-service', 'Browser service',
                 '{}'::jsonb, $4, '[]'::jsonb, '[]'::jsonb, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(agent_family)
    .bind(vec![4_u8; 32])
    .execute(worker)
    .await
    .expect("seed browser service release agent");
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
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'schema-managed', 'project', 'schema-managed', 'app',
                 'iframe', 'schema-managed', 'index.html', 1, 'no_store',
                 'managed_service')",
    )
    .bind(release)
    .execute(worker)
    .await
    .expect("seed managed UI descriptor");
    sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'schema-managed', 'browser-service', '/service', $2)",
    )
    .bind(release)
    .bind(release_agent)
    .execute(worker)
    .await
    .expect("seed managed UI binding");
    sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route,
          release_agent_id)
         VALUES ($1, 'schema-ui', 'status', 'browser-service', 'POST',
                 '/service/api', $2),
                ($1, 'schema-managed', 'status', 'browser-service', 'POST',
                 '/service/api', $2)",
    )
    .bind(release)
    .bind(release_agent)
    .execute(worker)
    .await
    .expect("seed API UI bindings");
    sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'schema-ui', 'index.html', $2, 'file', 'text/html'),
                ($1, 'schema-global', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(release)
    .bind(artifact)
    .execute(worker)
    .await
    .expect("seed static UI file");
    sqlx::query(
        "INSERT INTO gateways
         (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'browser-service', 'enabled', $4)",
    )
    .bind(managed_gateway)
    .bind(source_project)
    .bind(repository)
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed browser service gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'browser-service',
                 'http.service.v1', 'heph_authenticated', '{}'::jsonb,
                 ARRAY[]::text[], ARRAY[]::text[], 8080, '/ready', '/health',
                 'disabled', $7, $8)",
    )
    .bind(managed_revision)
    .bind(managed_gateway)
    .bind(source_project)
    .bind(repository)
    .bind(release)
    .bind(release_agent)
    .bind(vec![5_u8; 32])
    .bind(actor)
    .execute(worker)
    .await
    .expect("seed browser service revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/service', ARRAY['GET', 'POST'])",
    )
    .bind(Uuid::new_v4())
    .bind(managed_revision)
    .bind(managed_gateway)
    .bind(source_project)
    .execute(worker)
    .await
    .expect("seed browser service routes");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(managed_gateway)
        .bind(managed_revision)
        .execute(worker)
        .await
        .expect("activate browser service revision");
    if publish_release {
        sqlx::query(
            "UPDATE releases SET state = 'published', published_at = now(),
                    publication_actor_id = $2 WHERE id = $1",
        )
        .bind(release)
        .bind(actor)
        .execute(worker)
        .await
        .expect("publish release after descriptors");
    }

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
        source_project,
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
    seed_project_installation(
        worker,
        managed_installation,
        managed_generation,
        release,
        "schema-managed",
        actor,
        project,
    )
    .await;
    sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key,
          release_id, ui_key, gateway_id, gateway_revision_id,
          release_agent_id, gateway_name, method, route, exposure)
         VALUES
            ($1, $2, 'api', 'status', $3, 'schema-ui', $4, $5, $6,
             'browser-service', 'POST', '/service/api', 'heph_authenticated'),
            ($7, $8, 'managed_service', 'service', $3, 'schema-managed',
             $4, $5, $6, 'browser-service', 'GET', '/service',
             'heph_authenticated'),
            ($7, $8, 'api', 'status', $3, 'schema-managed', $4, $5, $6,
             'browser-service', 'POST', '/service/api', 'heph_authenticated')",
    )
    .bind(installation)
    .bind(generation)
    .bind(release)
    .bind(managed_gateway)
    .bind(managed_revision)
    .bind(release_agent)
    .bind(managed_installation)
    .bind(managed_generation)
    .execute(worker)
    .await
    .expect("seed static and managed/API installation bindings");

    Fixture {
        actor,
        outsider,
        organization,
        project,
        source_project,
        release,
        release_agent,
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
        managed_installation,
        managed_generation,
        managed_gateway,
        managed_revision,
        route: "schema-ui",
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

// The fixture helper keeps the installation identity tuple explicit at each
// call site so tests cannot accidentally mix repository and project scope.
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

// Handoff probes vary each persisted identity and both timestamps explicitly.
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

pub async fn insert_authenticated_child(
    pool: &PgPool,
    fixture: &Fixture,
    session_digest: Vec<u8>,
    child_expiry: &str,
) -> Uuid {
    let handoff = insert_handoff(
        pool,
        fixture,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        digest(90),
    )
    .await
    .expect("insert authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin authentication child");
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at, handoff.issued_at + $5::interval
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(session_digest)
    .bind(Uuid::new_v4())
    .bind(handoff)
    .bind(child_expiry)
    .execute(&mut *tx)
    .await
    .expect("insert authentication child");
    sqlx::query(
        "UPDATE ui_browser_handoffs
         SET consumed_at = statement_timestamp() WHERE id = $1",
    )
    .bind(handoff)
    .execute(&mut *tx)
    .await
    .expect("consume authentication handoff");
    tx.commit().await.expect("commit authentication child");
    child_id
}

pub async fn insert_managed_authenticated_child(
    pool: &PgPool,
    fixture: &Fixture,
    session_secret: [u8; 32],
) -> Uuid {
    let handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
         (id, handoff_digest, request_id, actor_id, parent_session_id,
          installation_id, generation_id, organization_id, route,
          issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'schema-managed',
                 statement_timestamp(), statement_timestamp() + interval '60 seconds')",
    )
    .bind(handoff)
    .bind(digest(205))
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(fixture.managed_installation)
    .bind(fixture.managed_generation)
    .bind(fixture.organization)
    .execute(pool)
    .await
    .expect("insert managed authentication handoff");
    let child_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin managed child");
    sqlx::query(
        "INSERT INTO ui_browser_sessions
         (id, session_digest, request_id, handoff_id, parent_session_id,
          installation_id, generation_id, organization_id, route, issued_at, expires_at)
         SELECT $1, $2, $3, handoff.id, handoff.parent_session_id,
                handoff.installation_id, handoff.generation_id, handoff.organization_id,
                handoff.route, handoff.issued_at, handoff.issued_at + interval '1 hour'
         FROM ui_browser_handoffs AS handoff WHERE handoff.id = $4",
    )
    .bind(child_id)
    .bind(
        UiBrowserSessionSecret::from_bytes(session_secret)
            .digest()
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(handoff)
    .execute(&mut *tx)
    .await
    .expect("insert managed authentication child");
    sqlx::query("UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1")
        .bind(handoff)
        .execute(&mut *tx)
        .await
        .expect("consume managed authentication handoff");
    tx.commit().await.expect("commit managed child");
    child_id
}

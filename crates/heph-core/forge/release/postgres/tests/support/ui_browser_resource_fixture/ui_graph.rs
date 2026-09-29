use super::seed::SeedIds;
use sqlx::PgPool;
use uuid::Uuid;

// Keep the complete UI/gateway graph together so its cross-table SQL remains
// auditable against the canonical release fixture.
#[allow(clippy::too_many_lines)]
pub async fn seed_ui_graph(worker: &PgPool, ids: &SeedIds, publish_release: bool) {
    let actor = ids.actor;
    let release = ids.release;
    let artifact = ids.artifact;
    let release_agent = ids.release_agent;
    let source_project = ids.source_project;
    let repository = ids.repository;
    let managed_gateway = ids.managed_gateway;
    let managed_revision = ids.managed_revision;
    for (ui_key, route_base, scope) in [
        ("schema-ui", "schema-ui", "project"),
        ("schema-ui-two", "schema-ui-two", "project"),
        ("schema-global", "schema-global", "global"),
        ("schema-repository", "schema-repository", "repository"),
    ] {
        sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind, repository_git_access)
             VALUES ($1, $2, $5, $3, 'app', 'iframe', $4,
                     'index.html', 1, 'no_store', 'static', $6)",
        )
        .bind(release)
        .bind(ui_key)
        .bind(ui_key)
        .bind(route_base)
        .bind(scope)
        .bind(if scope == "repository" {
            "read_write"
        } else {
            "none"
        })
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
                ($1, 'schema-global', 'index.html', $2, 'file', 'text/html'),
                ($1, 'schema-repository', 'index.html', $2, 'file', 'text/html')",
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
}

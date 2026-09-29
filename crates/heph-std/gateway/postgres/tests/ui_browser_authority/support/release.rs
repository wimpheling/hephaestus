//! Repository release and gateway rows used by UI authority scenarios.

use super::fixture::SeedIds;
use sqlx::PgPool;
use uuid::Uuid;

#[allow(clippy::too_many_lines)]
#[allow(clippy::cognitive_complexity)]
pub async fn seed_release(worker: &PgPool, ids: &SeedIds) {
    let SeedIds {
        actor,
        outsider,
        organization,
        other_organization,
        project,
        source_project,
        repository,
        receive,
        build,
        source_revision,
        release,
        artifact,
        managed_gateway,
        managed_revision,
        release_agent,
        agent_family,
        ..
    } = *ids;

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
                 'iframe', 'docs', 'index.html', 1, 'no_store',
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

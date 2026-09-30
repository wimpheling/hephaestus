use super::*;
use crate::fixture::FixtureSeedIds;

// These descriptors form one release fixture and are intentionally seeded in
// one ordered phase to preserve the database constraints under test.
#[allow(clippy::too_many_lines)]
pub async fn seed_release_descriptors(worker: &PgPool, ids: &FixtureSeedIds) {
    sqlx::query(
        "INSERT INTO releases
     (id, repository_id, version, source_commit, source_ref, build_request_id,
      build_definition_hash, configuration, configuration_hash, manifest_hash, state)
     VALUES ($1, $2, 'ui-browser-v1', $3, 'refs/heads/main', $4, $5,
             '{}'::jsonb, $6, $7, 'draft')",
    )
    .bind(ids.release)
    .bind(ids.repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(ids.build)
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
    .bind(ids.release)
    .bind(ids.build)
    .bind(ids.source_revision)
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
    .bind(ids.artifact)
    .bind(ids.release)
    .bind([7_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .execute(worker)
    .await
    .expect("seed static UI artifact");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
     VALUES ($1, $2, 'browser-service')",
    )
    .bind(ids.agent_family)
    .bind(ids.repository)
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
    .bind(ids.release_agent)
    .bind(ids.release)
    .bind(ids.agent_family)
    .bind(vec![4_u8; 32])
    .execute(worker)
    .await
    .expect("seed browser service release agent");
    for (ui_key, route_base, scope, presentation) in [
        ("schema-ui", "schema-ui", "project", "iframe"),
        ("schema-ui-two", "schema-ui-two", "project", "full_page"),
        ("schema-global", "schema-global", "global", "iframe"),
        (
            "schema-repository",
            "schema-repository",
            "repository",
            "iframe",
        ),
        (
            "schema-repository-no-git",
            "schema-repository-no-git",
            "repository",
            "iframe",
        ),
        (
            "schema-repository-write",
            "schema-repository-write",
            "repository",
            "iframe",
        ),
    ] {
        sqlx::query(
            "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind, repository_git_access)
         VALUES ($1, $2, $5, $3, 'app', $6, $4,
                 'index.html', 1, 'no_store', 'static',
                 CASE
                     WHEN $2 = 'schema-repository' THEN 'read'
                     WHEN $2 = 'schema-repository-write' THEN 'read_write'
                     ELSE 'none'
                 END)",
        )
        .bind(ids.release)
        .bind(ui_key)
        .bind(ui_key)
        .bind(route_base)
        .bind(scope)
        .bind(presentation)
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
    .bind(ids.release)
    .execute(worker)
    .await
    .expect("seed managed UI descriptor");
    sqlx::query(
        "INSERT INTO release_ui_managed_services
     (release_id, ui_key, gateway_name, route, release_agent_id)
     VALUES ($1, 'schema-managed', 'browser-service', '/service', $2)",
    )
    .bind(ids.release)
    .bind(ids.release_agent)
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
    .bind(ids.release)
    .bind(ids.release_agent)
    .execute(worker)
    .await
    .expect("seed API UI bindings");
    sqlx::query(
        "INSERT INTO release_ui_static_files
     (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
     VALUES ($1, 'schema-ui', 'index.html', $2, 'file', 'text/html'),
            ($1, 'schema-global', 'index.html', $2, 'file', 'text/html'),
            ($1, 'schema-repository', 'index.html', $2, 'file', 'text/html'),
            ($1, 'schema-repository-no-git', 'index.html', $2, 'file', 'text/html'),
            ($1, 'schema-repository-write', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(ids.release)
    .bind(ids.artifact)
    .execute(worker)
    .await
    .expect("seed static UI file");
}

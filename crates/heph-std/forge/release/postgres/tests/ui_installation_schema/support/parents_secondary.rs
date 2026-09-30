use sqlx::PgPool;
use uuid::Uuid;

// Keep a second fully valid publication identity so composite-FK failures cannot
// be attributed to ordinary missing-parent foreign keys.
#[allow(clippy::too_many_lines)]
pub async fn seed_second_published_fixture(
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

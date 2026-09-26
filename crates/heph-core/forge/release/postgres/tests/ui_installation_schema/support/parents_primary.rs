use sqlx::PgPool;
use uuid::Uuid;

use super::fixture::Fixture;
use super::parents_secondary::seed_second_published_fixture;

// This fixture deliberately seeds the complete publication graph in one setup
// transaction so every test exercises the same foreign-key topology.
#[allow(clippy::too_many_lines)]
pub async fn seed_parent_rows(pool: &PgPool) -> Fixture {
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

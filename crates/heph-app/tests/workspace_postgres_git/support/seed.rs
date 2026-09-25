use super::*;

// The fixture must seed one complete instance graph in one transaction so its
// foreign-key and capability assertions remain auditable together.
#[allow(clippy::too_many_lines)]
pub async fn seed_attached_instance(
    pool: &PgPool,
    project_id: forge_domain::ProjectId,
    repository_id: forge_domain::RepositoryId,
    commit: &str,
) {
    let configuration = agent_config(repository_id);
    let release_configuration = agent_config::parse(configuration.as_bytes())
        .config
        .expect("fixture release configuration should parse");
    let build_image_key = image_key(repository_id, "build");
    let runtime_image_key = image_key(repository_id, "runtime");
    let build_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin exact instance fixture");
    for key in [&build_image_key, &runtime_image_key] {
        sqlx::query(
            "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version)
             VALUES ($1, $2, $2, $3, '[]'::jsonb, ARRAY['x86_64'],
                     'available', '{}'::jsonb, 'workspace-test/v1')",
        )
        .bind(Uuid::new_v4())
        .bind(key)
        .bind(format!(
            "{key}@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ))
        .execute(&mut *tx)
        .await
        .expect("seed fixture OCI image");
    }
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())",
    )
    .bind(build_id)
    .bind(repository_id.as_uuid())
    .bind(commit)
    .bind([1_u8; 32].as_slice())
    .execute(&mut *tx)
    .await
    .expect("seed exact build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'workspace-agent')",
    )
    .bind(family_id)
    .bind(repository_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact family");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                 $8, $6, $7, 'published', now())",
    )
    .bind(release_id)
    .bind(repository_id.as_uuid())
    .bind(commit)
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .bind(serde_json::to_value(release_configuration).expect("serialize release configuration"))
    .execute(&mut *tx)
    .await
    .expect("seed exact release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'workspace-agent', 'Workspace Agent',
                 $4, $5, '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(serde_json::json!({
        "command": "bin/agent",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "fixture"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(&mut *tx)
    .await
    .expect("seed exact release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(instance_id)
    .bind(project_id.as_uuid())
    .bind(family_id)
    .bind(format!("workspace_{}", Uuid::new_v4().simple()))
    .execute(&mut *tx)
    .await
    .expect("seed exact instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable, diagnostics)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $5, $7,
                 'platform/test', true, '[]')",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({
        "vcpus": 1,
        "memory_mib": 128,
        "network": "disabled"
    }))
    .bind(serde_json::json!({"network": "disabled"}))
    .bind([6_u8; 32].as_slice())
    .execute(&mut *tx)
    .await
    .expect("seed exact revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(&mut *tx)
        .await
        .expect("activate exact revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy, enabled)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true)",
    )
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(project_id.as_uuid())
    .bind(repository_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact attachment");
    tx.commit().await.expect("commit exact instance fixture");
}

use super::*;

// Keeping the full immutable graph together makes this compatibility-breaking
// exact-trigger fixture easier to audit than a chain of partially valid rows.
#[allow(clippy::too_many_lines)]
pub async fn seed_attached_instance(
    pool: &PgPool,
    user_id: UserId,
    project_id: forge_domain::ProjectId,
    repository_id: forge_domain::RepositoryId,
    commit: &str,
) {
    let build_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin exact instance fixture");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, created_by, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5, now())",
    )
    .bind(build_id)
    .bind(repository_id.as_uuid())
    .bind(commit)
    .bind([1_u8; 32].as_slice())
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'reviewer')",
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
          configuration_hash, manifest_hash, state,
          publication_actor_id, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                 '{}', $6, $7, 'published', $8, now())",
    )
    .bind(release_id)
    .bind(repository_id.as_uuid())
    .bind(commit)
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'reviewer', 'Reviewer', $4, $5,
                 '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(json!({
        "command": "bin/reviewer",
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
         (id, project_id, family_id, name, state, created_by)
         VALUES ($1, $2, $3, $4, 'active', $5)",
    )
    .bind(instance_id)
    .bind(project_id.as_uuid())
    .bind(family_id)
    .bind(format!("reviewer_{}", Uuid::new_v4().simple()))
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable,
          diagnostics, created_by)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $5, $7,
                 'platform/test', true, '[]', $8)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 256, "network": "disabled"}))
    .bind(json!({"network": "disabled"}))
    .bind([6_u8; 32].as_slice())
    .bind(user_id.as_uuid())
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
          trigger_policy, enabled, created_by)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(project_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact attachment");
    tx.commit().await.expect("commit exact instance fixture");
}

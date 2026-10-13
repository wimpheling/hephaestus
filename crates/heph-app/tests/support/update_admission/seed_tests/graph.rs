//! Exact released-instance graph adapted from the `RunPG` fixture through the app façade.
use heph_run::StartRun;

// Keeping the complete exact-provenance graph together makes this fixture auditable.
#[allow(clippy::too_many_lines)]
pub async fn seed_instance(pool: &sqlx::PgPool, command: &StartRun) {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    let repository_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let build_request_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("runtime-{organization_id}"))
        .execute(pool)
        .await
        .expect("runtime organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("runtime-{project_id}"))
        .execute(pool)
        .await
        .expect("runtime project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, $3)",
    )
    .bind(repository_id)
    .bind(project_id)
    .bind(format!("runtime-{repository_id}"))
    .execute(pool)
    .await
    .expect("runtime repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'runtime')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("runtime family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())",
    )
    .bind(build_request_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8,
                 'published', now())",
    )
    .bind(command.release_id.as_uuid())
    .bind(repository_id)
    .bind(format!("test-{}", command.release_id))
    .bind("a".repeat(40))
    .bind(build_request_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'runtime', 'Runtime', '{}', $4, $5)",
    )
    .bind(command.release_agent_id.as_uuid())
    .bind(command.release_id.as_uuid())
    .bind(family_id)
    .bind([4_u8; 32].as_slice())
    .bind(command.requires_state)
    .execute(pool)
    .await
    .expect("runtime release agent");
    sqlx::query(
        "INSERT INTO agent_instances (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(command.instance_id.as_uuid())
    .bind(project_id)
    .bind(family_id)
    .bind(format!("runtime-{}", command.instance_id))
    .execute(pool)
    .await
    .expect("runtime instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'test/v1', true)",
    )
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.release_agent_id.as_uuid())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime revision");
    sqlx::query(
        "UPDATE agent_instances SET active_revision_id = $2
         WHERE id = $1",
    )
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .execute(pool)
    .await
    .expect("activate runtime revision");
    if let Some(attachment_id) = command.attachment_id {
        sqlx::query(
            "INSERT INTO agent_attachments
             (id, instance_id, project_id, repository_id, ref_selector,
              trigger_policy)
             VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual')",
        )
        .bind(attachment_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(project_id)
        .bind(repository_id)
        .execute(pool)
        .await
        .expect("runtime attachment");
    }
}

use super::candidate::seed_candidate;
use super::model::*;
use super::*;

// Keep the complete relational fixture together so each foreign-key edge is
// visible beside the Phase 1B scenario that consumes it.
#[allow(clippy::too_many_lines)]
pub async fn seed_instance(pool: &PgPool, instance_id: AgentInstanceId) -> UpdateScenario {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    let repository_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let build_id = uuid::Uuid::new_v4();
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let revision_id = AgentInstanceRevisionId::new();
    let attachment_id = AgentAttachmentId::new();
    let state_volume_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("libkrun-{organization_id}"))
        .execute(pool)
        .await
        .expect("libkrun organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("libkrun-{project_id}"))
        .execute(pool)
        .await
        .expect("libkrun project");
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public)
         VALUES ($1, $2, $3, 'refs/heads/main', false)",
    )
    .bind(repository_id)
    .bind(project_id)
    .bind(format!("repository-{repository_id}"))
    .execute(pool)
    .await
    .expect("libkrun repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'state-agent')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("libkrun family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("libkrun build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5, '{}',
                 $6, $7, 'published', now())",
    )
    .bind(release_id.as_uuid())
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("libkrun release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'state-agent', 'State Agent', $4, $5, true)",
    )
    .bind(release_agent_id.as_uuid())
    .bind(release_id.as_uuid())
    .bind(family_id)
    .bind(serde_json::json!({
        "command": "bin/agent",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "phase1b"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("libkrun release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(instance_id.as_uuid())
    .bind(project_id)
    .bind(family_id)
    .bind(format!("agent-{instance_id}"))
    .execute(pool)
    .await
    .expect("libkrun instance");
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
         (id, instance_id, state, capacity_bytes)
         VALUES ($1, $2, 'uninitialized', $3)",
    )
    .bind(state_volume_id)
    .bind(instance_id.as_uuid())
    .bind(128_i64 * 1024 * 1024)
    .execute(pool)
    .await
    .expect("libkrun instance state volume");
    sqlx::query("UPDATE agent_instances SET state_volume_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(state_volume_id)
        .execute(pool)
        .await
        .expect("attach libkrun instance state volume");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8, 'phase1b/v1', true)",
    )
    .bind(revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 512, "network": "egress"}))
    .bind(serde_json::json!({"network": "egress"}))
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 512, "network": "egress"}))
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("libkrun revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(pool)
        .await
        .expect("libkrun active revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual')",
    )
    .bind(attachment_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(project_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("libkrun attachment");
    let current = RunTarget {
        instance: instance_id,
        revision: revision_id,
        release: release_id,
        release_agent: release_agent_id,
        attachment: attachment_id,
    };
    UpdateScenario {
        current,
        successful: seed_candidate(pool, current, repository_id, family_id, "v2", 'b').await,
        rejected: seed_candidate(pool, current, repository_id, family_id, "v3", 'c').await,
        uncertain: seed_candidate(pool, current, repository_id, family_id, "v4", 'd').await,
    }
}

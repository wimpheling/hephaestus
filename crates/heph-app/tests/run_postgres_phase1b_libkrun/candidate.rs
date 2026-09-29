use super::model::*;
use super::*;

pub async fn seed_candidate(
    pool: &PgPool,
    current: RunTarget,
    repository_id: uuid::Uuid,
    family_id: uuid::Uuid,
    version: &str,
    commit_character: char,
) -> RunTarget {
    let build_id = uuid::Uuid::new_v4();
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let revision_id = AgentInstanceRevisionId::new();
    let commit = commit_character.to_string().repeat(40);
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind(&commit)
    .bind([11_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("candidate build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}',
                 $7, $8, 'published', now())",
    )
    .bind(release_id.as_uuid())
    .bind(repository_id)
    .bind(version)
    .bind(commit)
    .bind(build_id)
    .bind([11_u8; 32].as_slice())
    .bind([12_u8; 32].as_slice())
    .bind([13_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("candidate release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state, update_hook)
         VALUES ($1, $2, $3, 'state-agent', 'State Agent', $4, $5, true, $6)",
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
    .bind([14_u8; 32].as_slice())
    .bind(serde_json::json!({
        "command": "bin/update",
        "arguments": [],
        "working_directory": "."
    }))
    .execute(pool)
    .await
    .expect("candidate release agent");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8, 'phase1b/v2', true)",
    )
    .bind(revision_id.as_uuid())
    .bind(current.instance.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind([15_u8; 32].as_slice())
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 512, "network": "egress"}))
    .bind(serde_json::json!({"network": "egress"}))
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 512, "network": "egress"}))
    .bind([16_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("candidate instance revision");
    RunTarget {
        instance: current.instance,
        revision: revision_id,
        release: release_id,
        release_agent: release_agent_id,
        attachment: current.attachment,
    }
}

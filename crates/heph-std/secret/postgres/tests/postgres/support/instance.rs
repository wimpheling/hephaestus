use super::Fixture;
use runtime_types::RunId;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn seed_instance(
    pool: &PgPool,
    fixture: &Fixture,
) -> (
    release_domain::AgentInstanceId,
    release_domain::AgentInstanceRevisionId,
    Uuid,
    RunId,
) {
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let instance_id = release_domain::AgentInstanceId::new();
    let revision_id = release_domain::AgentInstanceRevisionId::new();
    let attachment_id = Uuid::new_v4();
    seed_release_graph(
        pool,
        fixture,
        build_id,
        release_id,
        release_agent_id,
        family_id,
    )
    .await;
    seed_instance_revision(
        pool,
        fixture,
        instance_id,
        revision_id,
        release_agent_id,
        family_id,
    )
    .await;
    seed_attachment(pool, fixture, instance_id, attachment_id).await;
    let run_id = seed_queued_run(pool, instance_id, revision_id, attachment_id).await;
    (instance_id, revision_id, attachment_id, run_id)
}

pub async fn seed_queued_run(
    pool: &PgPool,
    instance_id: release_domain::AgentInstanceId,
    revision_id: release_domain::AgentInstanceRevisionId,
    attachment_id: Uuid,
) -> RunId {
    let run_id = RunId::new();
    let provenance: (Uuid, Uuid) = sqlx::query_as(
        "SELECT agent.release_id, agent.id
          FROM agent_instance_revisions AS revision
          JOIN release_agents AS agent ON agent.id = revision.release_agent_id
          WHERE revision.id = $1 AND revision.instance_id = $2",
    )
    .bind(revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("run release provenance");
    sqlx::query(
        "INSERT INTO runs
          (id, instance_id, instance_revision_id, release_id, release_agent_id,
           attachment_id, run_kind, command_id, state, requires_state,
           created_at, updated_at)
          VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7, 'queued', true, now(), now())",
    )
    .bind(run_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(revision_id.as_uuid())
    .bind(provenance.0)
    .bind(provenance.1)
    .bind(attachment_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed queued run");
    run_id
}

pub async fn seed_release_graph(
    pool: &PgPool,
    fixture: &Fixture,
    build_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    family_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO build_requests
          (id, repository_id, source_commit, source_ref,
           build_definition_hash, state)
          VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(fixture.target_repository.as_uuid())
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
          VALUES ($1, $2, 'reviewer')",
    )
    .bind(family_id)
    .bind(fixture.target_repository.as_uuid())
    .execute(pool)
    .await
    .expect("seed agent family");
    sqlx::query(
        "INSERT INTO releases
          (id, repository_id, version, source_commit, source_ref,
           build_request_id, build_definition_hash, configuration,
           configuration_hash, manifest_hash, state, published_at)
          VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                  '{}', $6, $7, 'published', now())",
    )
    .bind(release_id)
    .bind(fixture.target_repository.as_uuid())
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release");
    sqlx::query(
        "INSERT INTO release_agents
          (id, release_id, family_id, agent_key, display_name,
           runtime_contract, runtime_contract_hash, parameter_schema,
           secret_slot_schema, requires_state)
          VALUES ($1, $2, $3, 'reviewer', 'Reviewer', $4, $5, '[]', $6, false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(json!({
        "policy_ceiling": {
            "vcpus": 2,
            "memory_mib": 1024,
            "network": "broker_only"
        }
    }))
    .bind([4_u8; 32].as_slice())
    .bind(json!([
        {
            "key": "model",
            "purpose": "Call model",
            "required": true,
            "delivery_modes": ["brokered", "raw"],
            "phases": ["normal", "update"],
            "destinations": ["api.example.test"]
        },
        {
            "key": "optional",
            "purpose": "Optional integration",
            "required": false,
            "delivery_modes": ["brokered"],
            "phases": ["normal", "update"],
            "destinations": ["api.example.test"]
        },
        {
            "key": "normal_only",
            "purpose": "Normal-run integration",
            "required": false,
            "delivery_modes": ["brokered"],
            "phases": ["normal"],
            "destinations": ["api.example.test"]
        },
        {
            "key": "update_only",
            "purpose": "Update-hook integration",
            "required": false,
            "delivery_modes": ["brokered"],
            "phases": ["update"],
            "destinations": ["api.example.test"]
        }
    ]))
    .execute(pool)
    .await
    .expect("seed release agent");
}

pub async fn seed_instance_revision(
    pool: &PgPool,
    fixture: &Fixture,
    instance_id: release_domain::AgentInstanceId,
    revision_id: release_domain::AgentInstanceRevisionId,
    release_agent_id: Uuid,
    family_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO agent_instances
          (id, project_id, family_id, name, state, active_revision_id,
           created_by)
          VALUES ($1, $2, $3, $4, 'active', NULL, $5)",
    )
    .bind(instance_id.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(family_id)
    .bind(format!("reviewer_{}", instance_id.as_uuid().simple()))
    .bind(fixture.target_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
          (id, instance_id, release_agent_id, parameters, parameter_hash,
           secret_bindings, resource_selection, network_restriction,
           effective_runtime_policy, effective_policy_hash,
           platform_policy_version, runnable, diagnostics, created_by)
          VALUES ($1, $2, $3, '{}', $4, '[]', $5, $6, $5, $7,
                  'platform/v1', false, $8, $9)",
    )
    .bind(revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 512, "network": "broker_only"}))
    .bind(json!({"network": "broker_only"}))
    .bind([6_u8; 32].as_slice())
    .bind(json!([{
        "code": "required_secret_binding_missing",
        "field": "secret_slots.model"
    }]))
    .bind(fixture.target_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed initial revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(pool)
        .await
        .expect("activate initial revision");
}

pub async fn seed_attachment(
    pool: &PgPool,
    fixture: &Fixture,
    instance_id: release_domain::AgentInstanceId,
    attachment_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO agent_attachments
          (id, instance_id, project_id, repository_id, ref_selector,
           trigger_policy, enabled, created_by)
          VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true, $5)",
    )
    .bind(attachment_id)
    .bind(instance_id.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.target_repository.as_uuid())
    .bind(fixture.target_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed attachment");
}

use super::*;

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn seed_reusable_instance(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    artifact_root: &Path,
    brokered_https: bool,
    artifact_override: Option<&[u8]>,
    instance_name: &str,
) -> SeededInstance {
    let build_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let release_id = uuid::Uuid::new_v4();
    let release_agent_id = uuid::Uuid::new_v4();
    let instance_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let attachment_id = uuid::Uuid::new_v4();
    let state_volume_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let cooking_artifact = cooking::agent_artifact();
    let artifact = artifact_override
        .or(cooking_artifact.as_deref())
        .unwrap_or(GOLDEN_AGENT.as_bytes());
    let release_configuration = serde_json::to_value(
        agent_config::parse(agent_config().as_bytes())
            .config
            .expect("golden reusable configuration should parse"),
    )
    .expect("serialize golden reusable configuration");
    tokio::fs::create_dir_all(artifact_root)
        .await
        .expect("release artifact root");
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("release artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("artifact metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();

    let secret_slot_schema = if artifact_override.is_some() {
        serde_json::json!([])
    } else if cooking::enabled() {
        cooking::secret_slots()
    } else if brokered_https {
        serde_json::json!([{
            "key": "model",
            "purpose": "Call a fixture HTTPS API",
            "required": true,
            "delivery_modes": ["brokered"],
            "phases": ["normal"],
            "destinations": ["api.example.test"]
        }])
    } else {
        serde_json::json!([])
    };
    sqlx::query(
        "INSERT INTO build_requests
           (id, repository_id, source_commit, source_ref,
            build_definition_hash, state, created_by, completed_at)
           VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded',
                   $5, now())",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
           VALUES ($1, $2, 'golden-agent')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("seed reusable family");
    sqlx::query(
        "INSERT INTO releases
           (id, repository_id, version, source_commit, source_ref,
           build_request_id, build_definition_hash, configuration,
            configuration_hash, manifest_hash, state, published_at)
           VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                   $6, $7, $8, 'published', now())",
    )
    .bind(release_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind(release_configuration)
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed reusable release");
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
           VALUES ($1, $2, 'bin/golden', 'executable', 365, $3, $4,
                   'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(release_id)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed reusable artifact");
    sqlx::query(
        "INSERT INTO release_agents
           (id, release_id, family_id, agent_key, display_name,
            runtime_contract, runtime_contract_hash, parameter_schema,
            secret_slot_schema, requires_state, update_hook)
           VALUES ($1, $2, $3, 'golden-agent', 'Golden Agent', $4, $5,
                   '[]', $6, true, $7)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/golden",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "root_image_digest": ROOT_IMAGE,
        "requires_state": true,
        "policy_ceiling": {
            "vcpus": 1,
            "memory_mib": 512,
            "network": if brokered_https { "broker_only" } else { "disabled" }
        }
    }))
    .bind([4_u8; 32].as_slice())
    .bind(secret_slot_schema)
    .bind(serde_json::json!({
        "command": "bin/golden",
        "arguments": [],
        "timeout_seconds": 60,
        "resources": {"vcpus": 1, "memory_mib": 512}
    }))
    .execute(pool)
    .await
    .expect("seed reusable release agent");
    sqlx::query(
        "INSERT INTO agent_instances
           (id, project_id, family_id, name, state, run_gate_open, created_by)
           VALUES ($1, $2, $3, $4, 'active', false, $5)",
    )
    .bind(instance_id)
    .bind(project_id)
    .bind(family_id)
    .bind(instance_name)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable instance");
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
           (id, instance_id, state, capacity_bytes)
           VALUES ($1, $2, 'uninitialized', $3)",
    )
    .bind(state_volume_id)
    .bind(instance_id)
    .bind(16_i64 * 1024 * 1024)
    .execute(pool)
    .await
    .expect("seed reusable instance state volume");
    sqlx::query("UPDATE agent_instances SET state_volume_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(state_volume_id)
        .execute(pool)
        .await
        .expect("attach reusable instance state volume");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
           (id, instance_id, release_agent_id, parameters, parameter_hash,
            secret_bindings, resource_selection, network_restriction,
            effective_runtime_policy, effective_policy_hash,
            platform_policy_version, runnable, diagnostics, created_by)
           VALUES ($1, $2, $3, $9, $4, '[]', $5, $6, $5, $7,
                   'platform/v1', true, '[]', $8)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({
        "vcpus": 1,
        "memory_mib": 512,
        "network": if brokered_https { "broker_only" } else { "disabled" }
    }))
    .bind(serde_json::json!({
        "network": if brokered_https { "broker_only" } else { "disabled" }
    }))
    .bind([6_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .bind(cooking::parameters())
    .execute(pool)
    .await
    .expect("seed reusable revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate reusable revision");
    sqlx::query(
        "INSERT INTO agent_attachments
           (id, instance_id, project_id, repository_id, ref_selector,
            trigger_policy, enabled, created_by)
           VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true, $5)",
    )
    .bind(attachment_id)
    .bind(instance_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable attachment");
    SeededInstance {
        instance: instance_id,
        revision: revision_id,
        attachment: attachment_id,
        release: release_id,
        release_agent: release_agent_id,
    }
}

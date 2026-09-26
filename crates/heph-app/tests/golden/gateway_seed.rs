use super::*;

pub struct SeededInstance {
    pub instance: uuid::Uuid,
    pub revision: uuid::Uuid,
    pub attachment: uuid::Uuid,
    pub release: uuid::Uuid,
    pub release_agent: uuid::Uuid,
}

pub struct ForgeRetryFixture {
    pub repository_id: uuid::Uuid,
    pub instance: SeededInstance,
    pub source_run_id: uuid::Uuid,
}

/// Exact host-side identity retained by the joined Caddy/libkrun mailbox
/// proof. The released guest never receives this identifier.
pub struct GatewayGoldenFixture {
    pub mailbox_id: MailboxId,
    pub grant_id: uuid::Uuid,
}

/// Exact durable declaration used by the opt-in daemon-to-Caddy service proof.
/// The desired pointer is set by the fixture, while the active pointer remains
/// unset until the production supervisor has observed HTTP readiness.
pub struct GatewayServiceGoldenFixture {
    pub gateway_id: uuid::Uuid,
    pub revision_id: uuid::Uuid,
}

pub type GatewayServiceCrashEvidence = (String, Option<String>, Option<i32>, Option<i32>);

/// Adds an exact released, stateless gateway handler alongside the reusable
/// agent. The artifact delegates only to the guest integration checker, which
/// validates that the daemon replaced the inbound secret before VM delivery.
pub async fn seed_gateway_release_agent(
    pool: &sqlx::PgPool,
    instance: &SeededInstance,
    artifact_root: &Path,
) -> uuid::Uuid {
    let agent_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let cooking_artifact = cooking::gateway_artifact();
    let artifact = cooking_artifact
        .as_deref()
        .unwrap_or(GATEWAY_HANDLER.as_bytes());
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("gateway release artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("gateway artifact metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("gateway artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
         VALUES ($1, $2, 'bin/gateway', 'executable', 365, $3, $4,
                 'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(instance.release)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("gateway artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed gateway release artifact");
    let family_id: uuid::Uuid =
        sqlx::query_scalar("SELECT family_id FROM release_agents WHERE id = $1")
            .bind(instance.release_agent)
            .fetch_one(pool)
            .await
            .expect("load reusable release family");
    sqlx::query(
        "INSERT INTO release_agents
           (id, release_id, family_id, agent_key, display_name,
            runtime_contract, runtime_contract_hash, parameter_schema,
            secret_slot_schema, requires_state, update_hook)
         VALUES ($1, $2, $3, 'golden-gateway', 'Golden gateway', $4, $5,
                 '[]', '[]', false, NULL)",
    )
    .bind(agent_id)
    .bind(instance.release)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/gateway",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "requires_state": false,
        "policy_ceiling": {"vcpus": 1, "memory_mib": 512, "network": "disabled"}
    }))
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed stateless gateway release agent");
    agent_id
}

use super::*;

/// Adds the long-lived integration-check service executable to the published
/// release. It shares the release with the stateless proof but uses a distinct
/// artifact path and release-agent key, so the two immutable contracts cannot
/// accidentally select one another.
pub async fn seed_gateway_service_release_agent(
    pool: &sqlx::PgPool,
    instance: &SeededInstance,
    artifact_root: &Path,
) -> uuid::Uuid {
    let agent_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let artifact = SERVICE_GATEWAY_HANDLER.as_bytes();
    tokio::fs::create_dir_all(artifact_root)
        .await
        .expect("service gateway release artifact root");
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("service gateway release artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("service gateway artifact metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("service gateway artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
         VALUES ($1, $2, 'bin/service', 'executable', 365, $3, $4,
                 'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(instance.release)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("service artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed service gateway release artifact");
    let family_id: uuid::Uuid =
        sqlx::query_scalar("SELECT family_id FROM release_agents WHERE id = $1")
            .bind(instance.release_agent)
            .fetch_one(pool)
            .await
            .expect("load reusable release family for service");
    sqlx::query(
        "INSERT INTO release_agents
           (id, release_id, family_id, agent_key, display_name,
            runtime_contract, runtime_contract_hash, parameter_schema,
            secret_slot_schema, requires_state, update_hook)
         VALUES ($1, $2, $3, 'golden-service', 'Golden service', $4, $5,
                 '[]', '[]', false, NULL)",
    )
    .bind(agent_id)
    .bind(instance.release)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/service",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "requires_state": false,
        "policy_ceiling": {"vcpus": 1, "memory_mib": 512, "network": "disabled"}
    }))
    .bind([11_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed service gateway release agent");
    agent_id
}

/// Seeds one public `http.service.v1` declaration without pre-starting it.
/// The daemon must claim the desired revision, reach readiness, and promote
/// the active pointer before public requests are attempted.
pub async fn seed_gateway_service_route(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    release_id: uuid::Uuid,
    release_agent_id: uuid::Uuid,
    capture_application_logs: bool,
) -> GatewayServiceGoldenFixture {
    let gateway_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let identity_route_id = uuid::Uuid::new_v4();
    let service_route_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateways
           (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'golden-service', 'enabled', $4)",
    )
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed service gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
           (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
            release_agent_key, handler_contract, exposure, parameters, secret_slots,
            mailbox_slots, normalized_hash, created_by, service_loopback_port,
            service_readiness_path, service_health_path, service_log_capture_mode)
         VALUES ($1, $2, $3, $4, $5, $6, 'golden-service', 'http.service.v1',
                 'public', '{}'::jsonb, '{}', '{}', $7, $8, 8080, '/readyz', '/healthz', $9)",
    )
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind([12_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .bind(if capture_application_logs {
        "application"
    } else {
        "disabled"
    })
    .execute(pool)
    .await
    .expect("seed service gateway revision");
    for (route_id, path) in [
        (service_route_id, "/service"),
        (identity_route_id, "/service/identity"),
        (uuid::Uuid::new_v4(), "/service/crash"),
        (uuid::Uuid::new_v4(), "/service/log"),
    ] {
        sqlx::query(
            "INSERT INTO gateway_routes
               (id, gateway_revision_id, gateway_id, project_id, path, methods)
             VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
        )
        .bind(route_id)
        .bind(revision_id)
        .bind(gateway_id)
        .bind(project_id)
        .bind(path)
        .execute(pool)
        .await
        .expect("seed service gateway route");
    }
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("publish service gateway desired revision");
    GatewayServiceGoldenFixture {
        gateway_id,
        revision_id,
    }
}

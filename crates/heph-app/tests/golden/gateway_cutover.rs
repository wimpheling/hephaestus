use super::*;

/// Creates a second independently published service release while leaving the
/// existing desired pointer untouched.  The cutover proof advances that
/// pointer only after the first public invocation has been accepted.
// The SQL fixture deliberately mirrors the published-release rows as one
// bounded setup operation; splitting it would obscure the immutable IDs.
#[allow(clippy::too_many_lines)]
pub async fn seed_gateway_service_cutover_candidate(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    artifact_root: &Path,
    readiness_path: &str,
) -> GatewayServiceGoldenFixture {
    let (project_id, repository_id, source_agent_key, source_publication_actor_id, created_by): (
        uuid::Uuid,
        uuid::Uuid,
        String,
        Option<uuid::Uuid>,
        uuid::Uuid,
    ) = sqlx::query_as(
        "SELECT gateway.project_id, gateway.repository_id,
                revision.release_agent_key, release.publication_actor_id,
                gateway.created_by
           FROM gateways AS gateway
           JOIN gateway_revisions AS revision
             ON revision.id = $2 AND revision.gateway_id = gateway.id
           JOIN releases AS release
             ON release.id = revision.release_id
          WHERE gateway.id = $1",
    )
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read published service source metadata");
    let publication_actor_id = source_publication_actor_id.unwrap_or(created_by);
    let authorized_actor: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT member.user_id
           FROM gateways AS gateway
           JOIN projects AS project ON project.id = gateway.project_id
           JOIN organization_members AS member
             ON member.organization_id = project.organization_id
            AND member.user_id = $2
          WHERE gateway.id = $1
            AND member.role IN ('owner', 'admin')",
    )
    .bind(fixture.gateway_id)
    .bind(publication_actor_id)
    .fetch_optional(pool)
    .await
    .expect("verify cutover publication actor authorization");
    assert_eq!(
        authorized_actor,
        Some(publication_actor_id),
        "cutover publication actor must belong to the gateway project organization"
    );
    let release_id = uuid::Uuid::new_v4();
    let build_request_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let release_agent_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let source_commit = format!("{:040x}", release_id.as_u128());
    let mut normalized_hash = [0_u8; 32];
    normalized_hash[..16].copy_from_slice(release_id.as_bytes());
    normalized_hash[16..].copy_from_slice(release_id.as_bytes());

    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref,
             build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_request_id)
    .bind(repository_id)
    .bind(&source_commit)
    .bind([21_u8; 32].as_slice())
    .bind(publication_actor_id)
    .execute(pool)
    .await
    .expect("seed cutover build request");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref,
             build_request_id, build_definition_hash, configuration,
             configuration_hash, manifest_hash, state,
             publication_actor_id, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}',
                 $7, $8, 'published', $9, now())",
    )
    .bind(release_id)
    .bind(repository_id)
    .bind(format!("cutover-{}", release_id.simple()))
    .bind(&source_commit)
    .bind(build_request_id)
    .bind([21_u8; 32].as_slice())
    .bind([22_u8; 32].as_slice())
    .bind([23_u8; 32].as_slice())
    .bind(publication_actor_id)
    .execute(pool)
    .await
    .expect("publish cutover release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family_id)
    .bind(repository_id)
    .bind(format!("cutover-agent-{}", release_id.simple()))
    .execute(pool)
    .await
    .expect("seed cutover agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name,
             runtime_contract, runtime_contract_hash, parameter_schema,
             secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, $4, 'Cutover service', $5, $6,
                 '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(&source_agent_key)
    .bind(serde_json::json!({
        "executable": "bin/service",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "requires_state": false,
        "policy_ceiling": {"vcpus": 1, "memory_mib": 512, "network": "disabled"}
    }))
    .bind([24_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed cutover release agent");

    let artifact = SERVICE_GATEWAY_HANDLER.as_bytes();
    tokio::fs::create_dir_all(artifact_root)
        .await
        .expect("cutover release artifact root");
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("cutover service artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("cutover service artifact metadata")
        .permissions();
    PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("cutover service artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
         VALUES ($1, $2, 'bin/service', 'executable', 365, $3, $4,
                 'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(release_id)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("cutover artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed cutover service artifact");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, mailbox_slots, normalized_hash, created_by,
             service_loopback_port, service_readiness_path, service_health_path)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'http.service.v1', 'public',
                 '{}', '{}', '{}', $8, $9, 8080, $10, '/healthz')",
    )
    .bind(revision_id)
    .bind(fixture.gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(&source_agent_key)
    .bind(normalized_hash.as_slice())
    .bind(publication_actor_id)
    .bind(readiness_path)
    .execute(pool)
    .await
    .expect("seed cutover service revision");
    for path in ["/service", "/service/identity", "/service/crash"] {
        sqlx::query(
            "INSERT INTO gateway_routes
               (id, gateway_revision_id, gateway_id, project_id, path, methods)
             VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(revision_id)
        .bind(fixture.gateway_id)
        .bind(project_id)
        .bind(path)
        .execute(pool)
        .await
        .expect("seed cutover service route");
    }
    GatewayServiceGoldenFixture {
        gateway_id: fixture.gateway_id,
        revision_id,
    }
}

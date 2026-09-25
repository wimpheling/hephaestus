use sqlx::PgPool;
use uuid::Uuid;

// Keep the relational fixture in one ordered function so its dependency chain and
// cleanup-target state remain visible together.
#[allow(clippy::too_many_lines)]
pub async fn seed_aged_cleaned_log(pool: &PgPool) -> Uuid {
    let user = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let build_request = Uuid::new_v4();
    let release = Uuid::new_v4();
    let family = Uuid::new_v4();
    let release_agent = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'retention test user')")
        .bind(user)
        .execute(pool)
        .await
        .expect("seed retention user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'retention test org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed retention organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'retention')")
        .bind(project)
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed retention project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'retention')")
        .bind(repository)
        .bind(project)
        .execute(pool)
        .await
        .expect("seed retention repository");
    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref, build_definition_hash,
             state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_request)
    .bind(repository)
    .bind("0000000000000000000000000000000000000001")
    .bind([4_u8; 32].as_slice())
    .bind(user)
    .execute(pool)
    .await
    .expect("seed retention build request");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref, build_request_id,
             build_definition_hash, configuration, configuration_hash, manifest_hash,
             state, publication_actor_id, published_at)
         VALUES ($1, $2, '1.0.0', $3, 'refs/heads/main', $4, $5, '{}'::jsonb,
                 $6, $7, 'published', $8, now())",
    )
    .bind(release)
    .bind(repository)
    .bind("0000000000000000000000000000000000000001")
    .bind(build_request)
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .bind(user)
    .execute(pool)
    .await
    .expect("seed retention release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'retention-agent')",
    )
    .bind(family)
    .bind(repository)
    .execute(pool)
    .await
    .expect("seed retention agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name, runtime_contract,
             runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'retention-agent', 'Retention agent', '{}'::jsonb,
                 $4, '[]'::jsonb, '[]'::jsonb, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(family)
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed retention release agent");
    sqlx::query(
        "INSERT INTO gateways
            (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'retention', 'enabled', $4)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(user)
    .execute(pool)
    .await
    .expect("seed retention gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
             release_agent_key, handler_contract, exposure, parameters, service_log_capture_mode,
             normalized_hash, created_by, service_loopback_port, service_readiness_path,
             service_health_path)
         VALUES ($1, $2, $3, $4, $5, $6, 'retention-agent', 'http.service.v1',
                 'public', '{}'::jsonb, 'application', $7, $8, 18081, '/ready', '/health')",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(release)
    .bind(release_agent)
    .bind(vec![0_u8; 32])
    .bind(user)
    .execute(pool)
    .await
    .expect("seed retention service revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(revision)
        .execute(pool)
        .await
        .expect("activate retention revision");
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid, fencing_token,
             vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
         VALUES ($1, $2, $3, 'retention-test-host', $4, 2,
                 $5, 'cleaned', clock_timestamp() + interval '1 hour',
                 clock_timestamp() - interval '2 days', clock_timestamp() - interval '2 days')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(owner)
    .bind(format!("gateway-service-{instance}"))
    .execute(pool)
    .await
    .expect("seed cleaned retention instance");
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             acknowledged_through, retained_bytes, retained_chunks,
             updated_at)
         VALUES ($1, $2, $3, $4, 1, 0, 3, 1,
                 clock_timestamp() - interval '25 hours')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(project)
    .execute(pool)
    .await
    .expect("seed aged retention epoch");
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             acknowledged_through, retained_bytes, retained_chunks,
             updated_at)
         VALUES ($1, $2, $3, $4, 2, 0, 0, 0,
                 clock_timestamp() - interval '25 hours')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(project)
    .execute(pool)
    .await
    .expect("seed empty aged retention epoch");
    sqlx::query(
        "INSERT INTO gateway_service_log_chunks
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             sequence, stream, observed_at, bytes, stored_at)
         VALUES ($1, $2, $3, $4, 1, 0, 'stdout',
                 clock_timestamp() - interval '25 hours', $5,
                 clock_timestamp() - interval '25 hours')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(project)
    .bind(vec![1_u8, 2, 3])
    .execute(pool)
    .await
    .expect("seed aged retention payload");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, retained_bytes, retained_chunks, retained_epochs)
         VALUES ($1, 3, 1, 2)",
    )
    .bind(project)
    .execute(pool)
    .await
    .expect("seed consistent retention usage");
    project
}

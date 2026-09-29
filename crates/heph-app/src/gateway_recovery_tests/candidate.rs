use super::*;

// This fixture intentionally assembles a second complete published release
// so revocation tests can keep the replacement independently eligible.
#[allow(clippy::too_many_lines)]
pub(super) async fn seed_service_candidate(pool: &sqlx::PgPool, fixture: Fixture) -> Uuid {
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let (_source_release_id, _source_release_agent_id, source_agent_key, repository_id): (
        Uuid,
        Uuid,
        String,
        Uuid,
    ) = sqlx::query_as(
        "SELECT release_id, release_agent_id, release_agent_key, repository_id
               FROM gateway_revisions
              WHERE id = $1",
    )
    .bind(fixture.revision)
    .fetch_one(pool)
    .await
    .expect("read source service revision");
    let release_id = Uuid::new_v4();
    let build_request = Uuid::new_v4();
    let agent_family = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
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
    .bind(build_request)
    .bind(repository_id)
    .bind(&source_commit)
    .bind([14_u8; 32].as_slice())
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("candidate build request");
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
    .bind(format!("candidate-{}", release_id.simple()))
    .bind(&source_commit)
    .bind(build_request)
    .bind([14_u8; 32].as_slice())
    .bind([15_u8; 32].as_slice())
    .bind([16_u8; 32].as_slice())
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("candidate release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(agent_family)
    .bind(repository_id)
    .bind(format!("candidate-agent-{}", release_id.simple()))
    .execute(pool)
    .await
    .expect("candidate agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name,
             runtime_contract, runtime_contract_hash, parameter_schema,
             secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, $4, 'Candidate service', '{}', $5, '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(agent_family)
    .bind(&source_agent_key)
    .bind([17_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("candidate release agent");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port,
             service_readiness_path, service_health_path, normalized_hash,
             created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'http.service.v1', 'public',
                 '{}', '{hook}', 18081, '/ready', '/health', $8, $9)",
    )
    .bind(revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(&source_agent_key)
    .bind(normalized_hash.as_slice())
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("candidate service revision");
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
    )
    .bind(route)
    .bind(revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(format!("/candidate-service-{}", revision.simple()))
    .execute(pool)
    .await
    .expect("candidate service route");
    revision
}

//! SQL row helpers shared by service target scenarios.

use super::gateway::Fixture;
use uuid::Uuid;

pub async fn insert_disabled_service_revision(
    pool: &sqlx::PgPool,
    revision: Uuid,
    gateway: Uuid,
    project: Uuid,
) {
    let (repository, owner): (Uuid, Uuid) = sqlx::query_as(
        "SELECT repository_id, created_by FROM gateways WHERE id = $1 AND project_id = $2",
    )
    .bind(gateway)
    .bind(project)
    .fetch_one(pool)
    .await
    .expect("disabled revision gateway identity");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, service_loopback_port, service_readiness_path,
             service_health_path, service_log_capture_mode, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, 'http.service.v1', 'public', '{}', 18080,
                 '/ready', '/health', 'disabled', $5, $6)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind([8_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("disabled service revision");
}

pub async fn insert_release(
    pool: &sqlx::PgPool,
    repository: Uuid,
    owner: Uuid,
    version: &str,
    state: &str,
) -> Uuid {
    let release = Uuid::new_v4();
    let build = Uuid::new_v4();
    let source_commit = format!("00000000{}", release.simple());
    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref, build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build)
    .bind(repository)
    .bind(&source_commit)
    .bind([4_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("build request");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref, build_request_id,
             build_definition_hash, configuration, configuration_hash, manifest_hash,
             state, publication_actor_id, published_at, revoked_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, $9, $10,
                 CASE WHEN $9 = 'published' THEN now() ELSE NULL END,
                 CASE WHEN $9 = 'revoked' THEN now() ELSE NULL END)",
    )
    .bind(release)
    .bind(repository)
    .bind(version)
    .bind(&source_commit)
    .bind(build)
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .bind(state)
    .bind(owner)
    .execute(pool)
    .await
    .expect("release");
    let family = Uuid::new_v4();
    let agent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family)
    .bind(repository)
    .bind(format!("target-agent-{release}"))
    .execute(pool)
    .await
    .expect("release agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name, runtime_contract,
             runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'target-service', 'Target service', '{}', $4, '[]', '[]', false)",
    )
    .bind(agent)
    .bind(release)
    .bind(family)
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("release agent");
    release
}

// The SQL fixture mirrors the revision identity columns directly for clarity.
#[allow(clippy::too_many_arguments)]
pub async fn insert_revision(
    pool: &sqlx::PgPool,
    revision: Uuid,
    gateway: Uuid,
    project: Uuid,
    repository: Uuid,
    owner: Uuid,
    release: Option<Uuid>,
    contract: &str,
    hash: [u8; 32],
) {
    let (port, readiness, health) = if contract == "http.service.v1" {
        (Some(18080_i32), Some("/ready"), Some("/health"))
    } else {
        (None, None, None)
    };
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
             release_agent_key, handler_contract, exposure, parameters,
             service_loopback_port, service_readiness_path, service_health_path,
             service_log_capture_mode, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5,
                 CASE WHEN $5 IS NULL THEN NULL ELSE
                     (SELECT id FROM release_agents WHERE release_id = $5 LIMIT 1)
                 END,
                 CASE WHEN $5 IS NULL THEN NULL ELSE 'target-service' END,
                 $6, 'public', '{}', $7, $8, $9,
                 CASE WHEN $6 = 'http.service.v1' THEN 'application' ELSE 'disabled' END,
                 $10, $11)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(release)
    .bind(contract)
    .bind(port)
    .bind(readiness)
    .bind(health)
    .bind(hash.as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway revision");
}

pub async fn set_pointers(pool: &sqlx::PgPool, gateway: Uuid, active: Uuid, desired: Option<Uuid>) {
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2, desired_service_revision_id = $3
          WHERE id = $1",
    )
    .bind(gateway)
    .bind(active)
    .bind(desired)
    .execute(pool)
    .await
    .expect("gateway pointers");
}

pub async fn insert_accepted_invocation(pool: &sqlx::PgPool, fixture: &Fixture) {
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id, project_id,
             request_id, outcome, service_instance_id,
             service_instance_fencing_token)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(fixture.old_route)
    .bind(fixture.project)
    .bind(Uuid::new_v4())
    .bind(fixture.old_instance)
    .bind(1_i64)
    .execute(pool)
    .await
    .expect("accepted invocation");
}

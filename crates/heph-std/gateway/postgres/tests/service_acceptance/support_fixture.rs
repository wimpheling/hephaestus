//! Service acceptance fixture and database query helpers.

use super::support_types::Fixture;
use uuid::Uuid;

pub async fn seed_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    seed_fixture_with_exposure(pool, contract, "public").await
}

pub async fn seed_fixture_with_exposure(
    pool: &sqlx::PgPool,
    contract: &str,
    exposure: &str,
) -> Fixture {
    seed_fixture_with_timing(pool, contract, true, 600, exposure).await
}

pub async fn seed_fixture_with_lease(
    pool: &sqlx::PgPool,
    contract: &str,
    lease_seconds: i64,
) -> Fixture {
    seed_fixture_with_timing(pool, contract, true, lease_seconds, "public").await
}

// This real-PostgreSQL fixture deliberately builds the release, agent,
// revision, route, and leased instance graph in one place.
#[allow(clippy::too_many_lines)]
pub async fn seed_fixture_with_expiry(
    pool: &sqlx::PgPool,
    contract: &str,
    lease_live: bool,
) -> Fixture {
    seed_fixture_with_timing(pool, contract, lease_live, 600, "public").await
}

// This real-PostgreSQL fixture deliberately builds the release, agent,
// revision, route, and leased instance graph in one place.
#[allow(clippy::too_many_lines)]
pub async fn seed_fixture_with_timing(
    pool: &sqlx::PgPool,
    contract: &str,
    lease_live: bool,
    lease_seconds: i64,
    exposure: &str,
) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let release = Uuid::new_v4();
    let build_request = Uuid::new_v4();
    let agent_family = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'acceptance owner')")
        .bind(owner)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("acceptance-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("acceptance-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("acceptance-{repository}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query(
        "INSERT INTO gateways
            (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, 'enabled', $5)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!("acceptance-{gateway}"))
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway");
    let service = contract == "http.service.v1";
    if service {
        let source_commit = format!("{:040x}", release.as_u128());
        sqlx::query(
            "INSERT INTO build_requests
                (id, repository_id, source_commit, source_ref,
                 build_definition_hash, state, created_by)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
        )
        .bind(build_request)
        .bind(repository)
        .bind(&source_commit)
        .bind([4_u8; 32].as_slice())
        .bind(owner)
        .execute(pool)
        .await
        .expect("build request");
        sqlx::query(
            "INSERT INTO releases
                (id, repository_id, version, source_commit, source_ref,
                 build_request_id, build_definition_hash, configuration,
                 configuration_hash, manifest_hash, state,
                 publication_actor_id, published_at)
             VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}',
                     $7, $8, 'published', $9, now())",
        )
        .bind(release)
        .bind(repository)
        .bind(format!("acceptance-{}", release.simple()))
        .bind(&source_commit)
        .bind(build_request)
        .bind([4_u8; 32].as_slice())
        .bind([5_u8; 32].as_slice())
        .bind([6_u8; 32].as_slice())
        .bind(owner)
        .execute(pool)
        .await
        .expect("published release");
        sqlx::query(
            "INSERT INTO agent_families (id, repository_id, agent_key)
             VALUES ($1, $2, $3)",
        )
        .bind(agent_family)
        .bind(repository)
        .bind(format!("acceptance-agent-{}", release.simple()))
        .execute(pool)
        .await
        .expect("agent family");
        sqlx::query(
            "INSERT INTO release_agents
                (id, release_id, family_id, agent_key, display_name,
                 runtime_contract, runtime_contract_hash, parameter_schema,
                 secret_slot_schema, requires_state)
             VALUES ($1, $2, $3, 'acceptance-service', 'Acceptance service',
                     '{}', $4, '[]', '[]', false)",
        )
        .bind(release_agent)
        .bind(release)
        .bind(agent_family)
        .bind([7_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("release agent");
    }
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5,
                 $6, $7, $8, $9, '{}', '{}', $10, $11, $12, $13, $14)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(if service { Some(release) } else { None })
    .bind(if service { Some(release_agent) } else { None })
    .bind(if service {
        Some("acceptance-service")
    } else {
        None
    })
    .bind(contract)
    .bind(exposure)
    .bind(service.then_some(18_080_i32))
    .bind(service.then_some("/ready"))
    .bind(service.then_some("/health"))
    .bind([9_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(revision)
        .execute(pool)
        .await
        .expect("active revision");
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
    )
    .bind(route)
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(format!("/{contract}"))
    .execute(pool)
    .await
    .expect("route");
    let service_instance = if service {
        let instance = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, 'acceptance-host', $4, 1,
                     $5, 'ready',
                     CASE WHEN $6 THEN now() + ($7::double precision * interval '1 second')
                          ELSE now() - interval '1 minute' END,
                     CASE WHEN $6 THEN now()
                          ELSE now() - interval '10 minutes' END)",
        )
        .bind(instance)
        .bind(gateway)
        .bind(revision)
        .bind(owner)
        .bind(format!("gateway-service-{instance}"))
        .bind(lease_live)
        .bind(lease_seconds)
        .execute(pool)
        .await
        .expect("ready service instance");
        Some(instance)
    } else {
        None
    };
    Fixture {
        gateway,
        revision,
        route,
        service_instance,
    }
}

pub async fn latest_invocation(pool: &sqlx::PgPool, route: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM gateway_invocations
          WHERE gateway_route_id = $1 ORDER BY accepted_at DESC, id DESC LIMIT 1",
    )
    .bind(route)
    .fetch_one(pool)
    .await
    .expect("latest invocation")
}

pub async fn insert_service_invocation(
    pool: &sqlx::PgPool,
    fixture: &Fixture,
    instance_id: Option<Uuid>,
    fencing_token: Option<i64>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    let project_id: Uuid = sqlx::query_scalar("SELECT project_id FROM gateways WHERE id = $1")
        .bind(fixture.gateway)
        .fetch_one(pool)
        .await?;
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id, project_id,
             request_id, outcome, service_instance_id,
             service_instance_fencing_token)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.route)
    .bind(project_id)
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(fencing_token)
    .execute(pool)
    .await
}

pub async fn invocation_outcome(pool: &sqlx::PgPool, route: Uuid) -> Option<&'static str> {
    let invocation = latest_invocation(pool, route).await;
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(invocation)
            .fetch_one(pool)
            .await
            .expect("invocation outcome");
    match outcome.as_str() {
        "accepted" => Some("accepted"),
        "rejected" => Some("rejected"),
        _ => None,
    }
}

pub async fn invocation_count(pool: &sqlx::PgPool, route: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM gateway_invocations WHERE gateway_route_id = $1")
        .bind(route)
        .fetch_one(pool)
        .await
        .expect("invocation count")
}

pub async fn session_count(pool: &sqlx::PgPool, invocation: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM gateway_runtime_authority_sessions WHERE invocation_id = $1",
    )
    .bind(invocation)
    .fetch_one(pool)
    .await
    .expect("session count")
}

pub async fn session_status(pool: &sqlx::PgPool, invocation: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT status FROM gateway_runtime_authority_sessions WHERE invocation_id = $1",
    )
    .bind(invocation)
    .fetch_optional(pool)
    .await
    .expect("session status")
}

pub async fn active_lease_count(pool: &sqlx::PgPool, invocation: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM gateway_secret_leases
          WHERE invocation_id = $1 AND status = 'active'",
    )
    .bind(invocation)
    .fetch_one(pool)
    .await
    .expect("active lease count")
}

use time::{Duration, OffsetDateTime};
use uuid::Uuid;

pub struct Fixture {
    pub owner: Uuid,
    pub organization: Uuid,
    pub project: Uuid,
    pub gateway: Uuid,
    pub revision: Uuid,
    pub route: Uuid,
    pub invocation: Uuid,
    pub service_instance: Option<Uuid>,
}

// Keep the complete foreign-key fixture in one helper so each assertion uses
// the same durable gateway shape.
#[allow(clippy::too_many_lines)]
pub async fn seed_fixture(pool: &sqlx::PgPool, handler_contract: &str) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let invocation = Uuid::new_v4();
    let hash = [9_u8; 32];

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Runtime authority test owner')")
        .bind(owner)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("runtime-authority-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("runtime-authority-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("runtime-authority-{repository}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query("INSERT INTO gateways (id, project_id, repository_id, name, lifecycle, created_by) VALUES ($1, $2, $3, $4, 'enabled', $5)")
        .bind(gateway)
        .bind(project)
        .bind(repository)
        .bind(format!("runtime-authority-{gateway}"))
        .bind(owner)
        .execute(pool)
        .await
        .expect("gateway");
    let secret_slots: Vec<&str> = if handler_contract == "http.service.v1" {
        vec!["hook"]
    } else {
        Vec::new()
    };
    let service_loopback_port = (handler_contract == "http.service.v1").then_some(18080_i32);
    let service_readiness_path = (handler_contract == "http.service.v1").then_some("/ready");
    let service_health_path = (handler_contract == "http.service.v1").then_some("/health");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, 'public', '{}', $6, $7, $8, $9, $10, $11)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(handler_contract)
    .bind(&secret_slots)
    .bind(service_loopback_port)
    .bind(service_readiness_path)
    .bind(service_health_path)
    .bind(hash.as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(revision)
        .execute(pool)
        .await
        .expect("active revision");
    sqlx::query("INSERT INTO gateway_routes (id, gateway_revision_id, gateway_id, project_id, path, methods) VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])")
        .bind(route)
        .bind(revision)
        .bind(gateway)
        .bind(project)
        .bind(if handler_contract == "http.service.v1" { "/service" } else { "/stateless" })
        .execute(pool)
        .await
        .expect("gateway route");
    let service_instance = if handler_contract == "http.service.v1" {
        let instance = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, 'runtime-authority-host', $4, 1,
                     $5, 'ready', now() + interval '10 minutes', now())",
        )
        .bind(instance)
        .bind(gateway)
        .bind(revision)
        .bind(owner)
        .bind(format!("gateway-service-{instance}"))
        .execute(pool)
        .await
        .expect("ready service instance");
        Some(instance)
    } else {
        None
    };
    sqlx::query("INSERT INTO gateway_invocations (id, gateway_id, gateway_revision_id, gateway_route_id, project_id, request_id, outcome, service_instance_id, service_instance_fencing_token) VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, $8)")
        .bind(invocation)
        .bind(gateway)
        .bind(revision)
        .bind(route)
        .bind(project)
        .bind(Uuid::new_v4())
        .bind(service_instance)
        .bind(service_instance.map(|_| 1_i64))
        .execute(pool)
        .await
        .expect("gateway invocation");
    Fixture {
        owner,
        organization,
        project,
        gateway,
        revision,
        route,
        invocation,
        service_instance,
    }
}

pub async fn seed_expired_host_sessions(
    pool: &sqlx::PgPool,
    fixture: &Fixture,
    count: usize,
) -> Vec<Uuid> {
    let mut transaction = pool.begin().await.expect("begin batched session fixture");
    let now = OffsetDateTime::now_utc();
    let issued_at = now - Duration::minutes(10);
    let expires_at = now - Duration::minutes(1);
    let hash = [9_u8; 32];
    let mut session_ids = Vec::with_capacity(count);
    for _ in 0..count {
        let invocation = Uuid::new_v4();
        let snapshot = Uuid::new_v4();
        let session = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO gateway_invocations
                    (id, gateway_id, gateway_revision_id, gateway_route_id, project_id,
                 request_id, outcome, accepted_at, service_instance_id,
                 service_instance_fencing_token)
             VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, $8, $9)",
        )
        .bind(invocation)
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .bind(fixture.route)
        .bind(fixture.project)
        .bind(Uuid::new_v4())
        .bind(issued_at)
        .bind(fixture.service_instance)
        .bind(fixture.service_instance.map(|_| 1_i64))
        .execute(&mut *transaction)
        .await
        .expect("batched gateway invocation");
        sqlx::query(
            "INSERT INTO gateway_authorization_snapshots
                (id, invocation_id, gateway_id, gateway_revision_id,
                 authorization_model_version, normalized_hash)
             VALUES ($1, $2, $3, $4, 'test/v1', $5)",
        )
        .bind(snapshot)
        .bind(invocation)
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .bind(hash.as_slice())
        .execute(&mut *transaction)
        .await
        .expect("batched gateway snapshot");
        sqlx::query(
            "INSERT INTO gateway_runtime_authority_sessions
                (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
                 identity_hash, snapshot_hash, issuance_generation, credential_hash,
                 admission_mode, status, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, NULL,
                     'host_mediated', 'active', $8, $9)",
        )
        .bind(session)
        .bind(snapshot)
        .bind(invocation)
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .bind(issued_at)
        .bind(expires_at)
        .execute(&mut *transaction)
        .await
        .expect("batched host session");
        session_ids.push(session);
    }
    transaction
        .commit()
        .await
        .expect("commit batched session fixture");
    session_ids
}

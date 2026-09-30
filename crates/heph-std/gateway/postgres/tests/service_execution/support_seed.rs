//! Service execution fixture graph construction.

use super::support_secret::seed_secret;
use super::support_types::{Fixture, SecretContext, TimingWindow};
use gateway_domain::GatewayServiceOwner;
use uuid::Uuid;

pub async fn seed_service(pool: &sqlx::PgPool) -> Fixture {
    seed_service_inner(
        pool,
        false,
        TimingWindow::Live,
        TimingWindow::Live,
        TimingWindow::Live,
    )
    .await
}

pub async fn seed_service_with_secret_lease(pool: &sqlx::PgPool) -> Fixture {
    seed_service_inner(
        pool,
        true,
        TimingWindow::Live,
        TimingWindow::Live,
        TimingWindow::Live,
    )
    .await
}

// Keep the complete authority graph in one fixture so each test exercises the
// same worker-visible rows and trigger boundaries.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn seed_service_inner(
    pool: &sqlx::PgPool,
    with_secret_lease: bool,
    session_timing: TimingWindow,
    instance_timing: TimingWindow,
    secret_lease_timing: TimingWindow,
) -> Fixture {
    let owner_id = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let release = Uuid::new_v4();
    let build = Uuid::new_v4();
    let family = Uuid::new_v4();
    let agent = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let invocation = Uuid::new_v4();
    let request = Uuid::new_v4();
    let snapshot = Uuid::new_v4();
    let session = Uuid::new_v4();
    let daemon = Uuid::new_v4();
    let secret = Uuid::new_v4();
    let version = Uuid::new_v4();
    let grant = Uuid::new_v4();
    let import = Uuid::new_v4();
    let binding = Uuid::new_v4();
    let rule = Uuid::new_v4();
    let secret_lease = Uuid::new_v4();
    let owner = GatewayServiceOwner::new("execution-host", daemon).expect("owner");

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'execution owner')")
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("execution-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("execution-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("execution-{repository}"))
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
    .bind(format!("execution-{}", &gateway.to_string()[..8]))
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("gateway");

    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref,
             build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build)
    .bind(repository)
    .bind(format!("{:040x}", release.as_u128()))
    .bind([4_u8; 32].as_slice())
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("build");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref,
             build_request_id, build_definition_hash, configuration,
             configuration_hash, manifest_hash, state, publication_actor_id,
             published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8,
                 'published', $9, now())",
    )
    .bind(release)
    .bind(repository)
    .bind(format!("execution-{release}"))
    .bind(format!("{:040x}", release.as_u128()))
    .bind(build)
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family)
    .bind(repository)
    .bind(format!("execution-family-{release}"))
    .execute(pool)
    .await
    .expect("family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name,
             runtime_contract, runtime_contract_hash, parameter_schema,
             secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'execution-service', 'Execution service', '{}',
                 $4, '[]', '[]', false)",
    )
    .bind(agent)
    .bind(release)
    .bind(family)
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("agent");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port,
             service_readiness_path, service_health_path, normalized_hash,
             created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'http.service.v1', 'public',
                 '{}', $8, 18080, '/ready', '/health', $9, $10)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(release)
    .bind(agent)
    .bind("execution-service")
    .bind(if with_secret_lease {
        vec![String::from("hook")]
    } else {
        Vec::new()
    })
    .bind([9_u8; 32].as_slice())
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("revision");
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/execute', ARRAY['GET'])",
    )
    .bind(route)
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .execute(pool)
    .await
    .expect("route");
    let instance_query = match instance_timing {
        TimingWindow::Live => sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, $4, $5, 1, $6, 'ready',
                     now() + interval '10 minutes', now())",
        ),
        TimingWindow::Near => sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, $4, $5, 1, $6, 'ready',
                     now() + interval '1 millisecond', now())",
        ),
        TimingWindow::Expired => sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, $4, $5, 1, $6, 'ready',
                     now() - interval '1 minute',
                     now() - interval '2 minutes')",
        ),
    };
    instance_query
        .bind(instance)
        .bind(gateway)
        .bind(revision)
        .bind(&owner.host_id)
        .bind(owner.owner_uuid)
        .bind(format!("gateway-service-{instance}"))
        .execute(pool)
        .await
        .expect("service instance");
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id,
             project_id, request_id, outcome, service_instance_id,
             service_instance_fencing_token)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, 1)",
    )
    .bind(invocation)
    .bind(gateway)
    .bind(revision)
    .bind(route)
    .bind(project)
    .bind(request)
    .bind(instance)
    .execute(pool)
    .await
    .expect("invocation");
    sqlx::query(
        "INSERT INTO gateway_authorization_snapshots
            (id, invocation_id, gateway_id, gateway_revision_id,
             authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'test/v1', $5)",
    )
    .bind(snapshot)
    .bind(invocation)
    .bind(gateway)
    .bind(revision)
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("snapshot");
    let session_query = match session_timing {
        TimingWindow::Live => sqlx::query(
            "INSERT INTO gateway_runtime_authority_sessions
                (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
                 identity_hash, snapshot_hash, issuance_generation, credential_hash,
                 admission_mode, status, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, NULL,
                     'host_mediated', 'active', now(),
                     now() + interval '10 minutes')",
        ),
        TimingWindow::Near => sqlx::query(
            "INSERT INTO gateway_runtime_authority_sessions
                (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
                 identity_hash, snapshot_hash, issuance_generation, credential_hash,
                 admission_mode, status, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, NULL,
                     'host_mediated', 'active', now(),
                     now() + interval '1 millisecond')",
        ),
        TimingWindow::Expired => sqlx::query(
            "INSERT INTO gateway_runtime_authority_sessions
                (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
                 identity_hash, snapshot_hash, issuance_generation, credential_hash,
                 admission_mode, status, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, NULL,
                     'host_mediated', 'active',
                     now() - interval '2 minutes',
                     now() - interval '1 minute')",
        ),
    };
    session_query
        .bind(session)
        .bind(snapshot)
        .bind(invocation)
        .bind(gateway)
        .bind(revision)
        .bind([2_u8; 32].as_slice())
        .bind([1_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("host-mediated session");

    if with_secret_lease {
        seed_secret(
            pool,
            SecretContext {
                organization,
                project,
                gateway,
                revision,
                route,
                owner_id,
                secret,
                version,
                grant,
                import,
                binding,
                rule,
                secret_lease,
                session,
                invocation,
            },
            secret_lease_timing,
        )
        .await;
    }

    Fixture {
        project,
        gateway,
        revision,
        route,
        instance,
        invocation,
        session,
        release,
        owner,
        secret_lease,
    }
}

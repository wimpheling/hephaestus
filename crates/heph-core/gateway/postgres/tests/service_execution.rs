//! Real `PostgreSQL` coverage for immutable gateway execution-target lookup.

use gateway_edge::{
    GatewayExecutionTarget, GatewayExecutionTargetError, GatewayExecutionTargetResolver,
    GatewayServiceOwner,
};
use gateway_postgres::PostgresGatewayExecutionTargetResolver;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, time::Duration};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execution_target_resolves_service_and_stateless_targets() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_service(&pool).await;
    let resolver = PostgresGatewayExecutionTargetResolver::new(worker_pool().await);
    let target = resolver
        .resolve_execution_target(
            fixture.invocation,
            fixture.route,
            fixture.revision,
            &fixture.owner,
        )
        .await
        .expect("service target");
    let GatewayExecutionTarget::Service(budget) = target else {
        panic!("service invocation resolved as stateless");
    };
    assert_eq!(budget.instance.identity.instance_id, fixture.instance);
    assert_eq!(budget.instance.identity.gateway_id, fixture.gateway);
    assert_eq!(budget.instance.identity.revision_id, fixture.revision);
    assert_eq!(budget.instance.fencing_token, 1);
    assert!(budget.remaining > Duration::from_secs(1));
    assert!(budget.remaining <= Duration::from_secs(600));

    let stateless = seed_stateless(&pool, fixture.gateway, fixture.project).await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(stateless.revision)
        .execute(&pool)
        .await
        .expect("cut over gateway active pointer");
    sqlx::query("UPDATE gateway_service_instances SET state = 'draining' WHERE id = $1")
        .bind(fixture.instance)
        .execute(&pool)
        .await
        .expect("drain accepted service instance");
    let draining = resolver
        .resolve_execution_target(
            fixture.invocation,
            fixture.route,
            fixture.revision,
            &fixture.owner,
        )
        .await
        .expect("draining old target remains executable");
    assert!(matches!(draining, GatewayExecutionTarget::Service(_)));

    let target = resolver
        .resolve_execution_target(
            stateless.invocation,
            stateless.route,
            stateless.revision,
            &fixture.owner,
        )
        .await
        .expect("stateless target");
    assert_eq!(target, GatewayExecutionTarget::Stateless);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execution_target_rejects_wrong_binding_and_exact_identity() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_service(&pool).await;
    let resolver = PostgresGatewayExecutionTargetResolver::new(worker_pool().await);
    let wrong_host = GatewayServiceOwner::new("other-host", fixture.owner.owner_uuid)
        .expect("same daemon on wrong host");
    let wrong_daemon = GatewayServiceOwner::new(fixture.owner.host_id.clone(), Uuid::new_v4())
        .expect("different daemon on same host");
    for (route, revision, owner) in [
        (fixture.route, fixture.revision, wrong_host),
        (fixture.route, fixture.revision, wrong_daemon),
        (Uuid::new_v4(), fixture.revision, fixture.owner.clone()),
        (fixture.route, Uuid::new_v4(), fixture.owner.clone()),
    ] {
        assert_eq!(
            resolver
                .resolve_execution_target(fixture.invocation, route, revision, &owner)
                .await,
            Err(GatewayExecutionTargetError::Unavailable)
        );
    }
    sqlx::query(
        "UPDATE gateway_invocations
            SET service_instance_fencing_token = 2
          WHERE id = $1",
    )
    .bind(fixture.invocation)
    .execute(&pool)
    .await
    .expect_err("immutable binding must reject a fence mutation");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execution_target_rejects_terminal_session_expiry_and_release_revocation() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let resolver = PostgresGatewayExecutionTargetResolver::new(worker_pool().await);

    let expired = seed_service(&pool).await;
    sqlx::query(
        "UPDATE gateway_runtime_authority_sessions
            SET status = 'expired'
          WHERE id = $1",
    )
    .bind(expired.session)
    .execute(&pool)
    .await
    .expect("expire host session");
    assert_unavailable(&resolver, &expired).await;

    let revoked = seed_service(&pool).await;
    sqlx::query(
        "UPDATE gateway_runtime_authority_sessions
            SET status = 'revoked', revoked_at = now(), revocation_reason = 'test'
          WHERE id = $1",
    )
    .bind(revoked.session)
    .execute(&pool)
    .await
    .expect("revoke host session");
    assert_unavailable(&resolver, &revoked).await;

    let terminal = seed_service(&pool).await;
    sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, 'timed_out')")
        .bind(terminal.invocation)
        .fetch_one(&pool)
        .await
        .expect("complete invocation");
    assert_unavailable(&resolver, &terminal).await;

    let release = seed_service(&pool).await;
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(release.release)
        .execute(&pool)
        .await
        .expect("revoke release");
    assert_unavailable(&resolver, &release).await;

    let expired_session = seed_service_inner(
        &pool,
        false,
        TimingWindow::Expired,
        TimingWindow::Live,
        TimingWindow::Live,
    )
    .await;
    assert_unavailable(&resolver, &expired_session).await;

    let expired_instance = seed_service_inner(
        &pool,
        false,
        TimingWindow::Live,
        TimingWindow::Expired,
        TimingWindow::Live,
    )
    .await;
    assert_unavailable(&resolver, &expired_instance).await;

    let near_expiry = seed_service_inner(
        &pool,
        false,
        TimingWindow::Near,
        TimingWindow::Live,
        TimingWindow::Live,
    )
    .await;
    assert_unavailable(&resolver, &near_expiry).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execution_target_rejects_revoked_exact_inbound_secret_lease() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_service_with_secret_lease(&pool).await;
    let resolver = PostgresGatewayExecutionTargetResolver::new(worker_pool().await);
    resolver
        .resolve_execution_target(
            fixture.invocation,
            fixture.route,
            fixture.revision,
            &fixture.owner,
        )
        .await
        .expect("active inbound lease permits target");
    sqlx::query(
        "UPDATE gateway_secret_leases
            SET status = 'revoked', revoked_at = now()
          WHERE id = $1",
    )
    .bind(fixture.secret_lease)
    .execute(&pool)
    .await
    .expect("revoke inbound lease");
    assert_unavailable(&resolver, &fixture).await;

    let expired = seed_service_inner(
        &pool,
        true,
        TimingWindow::Live,
        TimingWindow::Live,
        TimingWindow::Expired,
    )
    .await;
    assert_unavailable(&resolver, &expired).await;
}

async fn assert_unavailable(resolver: &PostgresGatewayExecutionTargetResolver, fixture: &Fixture) {
    assert_eq!(
        resolver
            .resolve_execution_target(
                fixture.invocation,
                fixture.route,
                fixture.revision,
                &fixture.owner,
            )
            .await,
        Err(GatewayExecutionTargetError::Unavailable)
    );
}

#[derive(Clone)]
struct Fixture {
    project: Uuid,
    gateway: Uuid,
    revision: Uuid,
    route: Uuid,
    instance: Uuid,
    invocation: Uuid,
    session: Uuid,
    release: Uuid,
    owner: GatewayServiceOwner,
    secret_lease: Uuid,
}

async fn seed_service(pool: &sqlx::PgPool) -> Fixture {
    seed_service_inner(
        pool,
        false,
        TimingWindow::Live,
        TimingWindow::Live,
        TimingWindow::Live,
    )
    .await
}

async fn seed_service_with_secret_lease(pool: &sqlx::PgPool) -> Fixture {
    seed_service_inner(
        pool,
        true,
        TimingWindow::Live,
        TimingWindow::Live,
        TimingWindow::Live,
    )
    .await
}

#[derive(Clone, Copy)]
enum TimingWindow {
    Live,
    Near,
    Expired,
}

// Keep the complete authority graph in one fixture so each test exercises the
// same worker-visible rows and trigger boundaries.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
async fn seed_service_inner(
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
        sqlx::query(
            "INSERT INTO secrets
                (id, owner_organization_id, project_id, name, status,
                 allowed_delivery_modes, policy, created_by)
             VALUES ($1, $2, $3, $4, 'active', ARRAY['brokered'], '{}', $5)",
        )
        .bind(secret)
        .bind(organization)
        .bind(project)
        .bind(format!("execution-secret-{secret}"))
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("secret");
        sqlx::query(
            "INSERT INTO secret_versions
                (id, secret_id, sequence, status, algorithm, key_reference,
                 data_nonce, ciphertext, wrap_nonce, wrapped_data_key,
                 associated_data_hash, content_length, created_by)
             VALUES ($1, $2, 1, 'active', 'AES-256-GCM+AES-256-GCM-KW/v1',
                     'execution-key', $3, $4, $5, $6, $7, 1, $8)",
        )
        .bind(version)
        .bind(secret)
        .bind([1_u8; 12].as_slice())
        .bind([2_u8; 1].as_slice())
        .bind([3_u8; 12].as_slice())
        .bind([4_u8; 32].as_slice())
        .bind([5_u8; 32].as_slice())
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("secret version");
        sqlx::query("UPDATE secrets SET active_version_id = $2 WHERE id = $1")
            .bind(secret)
            .bind(version)
            .execute(pool)
            .await
            .expect("active version");
        sqlx::query(
            "INSERT INTO secret_grants
                (id, secret_id, owner_organization_id, target_kind, target_id,
                 target_project_id, delivery_modes, phases, status, created_by)
             VALUES ($1, $2, $3, 'project', $4, $4, ARRAY['brokered'],
                     ARRAY['normal'], 'active', $5)",
        )
        .bind(grant)
        .bind(secret)
        .bind(organization)
        .bind(project)
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("secret grant");
        sqlx::query(
            "INSERT INTO secret_imports
                (id, grant_id, secret_id, target_kind, target_id, alias,
                 status, accepted_by)
             VALUES ($1, $2, $3, 'project', $4, $5, 'active', $6)",
        )
        .bind(import)
        .bind(grant)
        .bind(secret)
        .bind(project)
        .bind(format!("execution-import-{import}"))
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("secret import");
        sqlx::query(
            "INSERT INTO gateway_secret_bindings
                (id, gateway_id, gateway_revision_id, import_id, slot_key,
                 secret_version_id, status, normalized_hash)
             VALUES ($1, $2, $3, $4, 'hook', $5, 'active', $6)",
        )
        .bind(binding)
        .bind(gateway)
        .bind(revision)
        .bind(import)
        .bind(version)
        .bind([6_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("gateway secret binding");
        sqlx::query(
            "INSERT INTO gateway_brokered_secret_rules
                (id, binding_id, gateway_revision_id, gateway_route_id,
                 header_name, normalized_hash)
             VALUES ($1, $2, $3, $4, 'x-hook', $5)",
        )
        .bind(rule)
        .bind(binding)
        .bind(revision)
        .bind(route)
        .bind([7_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("gateway secret rule");
        let lease_query = match secret_lease_timing {
            TimingWindow::Live => sqlx::query(
                "INSERT INTO gateway_secret_leases
                    (id, runtime_session_id, invocation_id, binding_id,
                     secret_version_id, rule_id, status, issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'active', now(),
                         now() + interval '5 minutes')",
            ),
            TimingWindow::Near => sqlx::query(
                "INSERT INTO gateway_secret_leases
                    (id, runtime_session_id, invocation_id, binding_id,
                     secret_version_id, rule_id, status, issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'active', now(),
                         now() + interval '1 millisecond')",
            ),
            TimingWindow::Expired => sqlx::query(
                "INSERT INTO gateway_secret_leases
                    (id, runtime_session_id, invocation_id, binding_id,
                     secret_version_id, rule_id, status, issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'active',
                         now() - interval '2 minutes',
                         now() - interval '1 minute')",
            ),
        };
        lease_query
            .bind(secret_lease)
            .bind(session)
            .bind(invocation)
            .bind(binding)
            .bind(version)
            .bind(rule)
            .execute(pool)
            .await
            .expect("gateway secret lease");
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

async fn seed_stateless(pool: &sqlx::PgPool, gateway: Uuid, project: Uuid) -> Fixture {
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let invocation = Uuid::new_v4();
    let repository: Uuid = sqlx::query_scalar("SELECT repository_id FROM gateways WHERE id = $1")
        .bind(gateway)
        .fetch_one(pool)
        .await
        .expect("repository");
    let owner: Uuid = sqlx::query_scalar("SELECT created_by FROM gateways WHERE id = $1")
        .bind(gateway)
        .fetch_one(pool)
        .await
        .expect("gateway owner");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract,
             exposure, parameters, secret_slots, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, 'http.v1', 'public', '{}', '{}', $5, $6)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind([8_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("stateless revision");
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/stateless', ARRAY['GET'])",
    )
    .bind(route)
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .execute(pool)
    .await
    .expect("stateless route");
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id,
             project_id, request_id, outcome)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted')",
    )
    .bind(invocation)
    .bind(gateway)
    .bind(revision)
    .bind(route)
    .bind(project)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("stateless invocation");
    Fixture {
        project,
        gateway,
        revision,
        route,
        instance: Uuid::nil(),
        invocation,
        session: Uuid::nil(),
        release: Uuid::nil(),
        owner: GatewayServiceOwner::new("execution-host", Uuid::new_v4()).expect("owner"),
        secret_lease: Uuid::nil(),
    }
}

async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest migration");
    assert!(max_version >= 75);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

async fn worker_pool() -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-execution-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker PostgreSQL")
}

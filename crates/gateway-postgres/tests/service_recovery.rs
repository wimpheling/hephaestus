//! Real `PostgreSQL` coverage for bounded service-invocation recovery.

use gateway_edge::GatewayLimits;
use gateway_postgres::PostgresGatewayEdgeAuthority;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, time::Duration as StdDuration};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

#[derive(Clone, Copy)]
struct Fixture {
    owner: Uuid,
    organization: Uuid,
    project: Uuid,
    gateway: Uuid,
    revision: Uuid,
    route: Uuid,
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_recovery_terminalizes_only_abandoned_host_invocations() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let service = seed_fixture(&pool, "http.service.v1").await;
    let authority = authority(pool.clone());

    let expired = insert_invocation(&pool, service, now - Duration::minutes(10)).await;
    let expired_session =
        insert_host_session(&pool, service, expired, now, now + Duration::minutes(10)).await;
    let expired_lease = insert_lease(&pool, service, expired, expired_session).await;
    sqlx::query(
        "UPDATE gateway_runtime_authority_sessions
            SET status = 'expired'
          WHERE id = $1",
    )
    .bind(expired_session)
    .execute(&pool)
    .await
    .expect("expire host session");

    let revoked = insert_invocation(&pool, service, now - Duration::minutes(10)).await;
    let revoked_session =
        insert_host_session(&pool, service, revoked, now, now + Duration::minutes(10)).await;
    sqlx::query(
        "UPDATE gateway_runtime_authority_sessions
            SET status = 'revoked', revoked_at = $2, revocation_reason = 'test-recovery'
          WHERE id = $1",
    )
    .bind(revoked_session)
    .bind(now)
    .execute(&pool)
    .await
    .expect("revoke host session");

    let old_without_session = insert_invocation(&pool, service, now - Duration::minutes(10)).await;
    let active_expired = insert_invocation(&pool, service, now - Duration::minutes(10)).await;
    let active_expired_session = insert_host_session(
        &pool,
        service,
        active_expired,
        now,
        now - Duration::minutes(1),
    )
    .await;
    let live = insert_invocation(&pool, service, now - Duration::minutes(10)).await;
    let live_session =
        insert_host_session(&pool, service, live, now, now + Duration::minutes(10)).await;
    let stateless = seed_fixture(&pool, "http.v1").await;
    let stateless_old = insert_invocation(&pool, stateless, now - Duration::minutes(10)).await;

    assert_eq!(
        authority
            .recover_abandoned_service_invocations(now)
            .await
            .expect("recover abandoned service invocations"),
        4
    );
    assert_eq!(outcome(&pool, expired).await, "timed_out");
    assert_eq!(outcome(&pool, revoked).await, "timed_out");
    assert_eq!(outcome(&pool, old_without_session).await, "timed_out");
    assert_eq!(outcome(&pool, active_expired).await, "timed_out");
    assert_eq!(
        session_status(&pool, active_expired_session).await,
        "revoked"
    );
    assert_eq!(outcome(&pool, live).await, "accepted");
    assert_eq!(outcome(&pool, stateless_old).await, "accepted");
    assert_eq!(session_status(&pool, live_session).await, "active");
    assert_eq!(lease_status(&pool, expired_lease).await, "revoked");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_recovery_is_bounded_and_skips_locked_invocations() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let authority = authority(pool.clone());
    let mut invocations = Vec::with_capacity(129);
    for _ in 0..129 {
        invocations.push(insert_invocation(&pool, fixture, now - Duration::minutes(10)).await);
    }

    let mut lock = pool.begin().await.expect("begin invocation lock");
    sqlx::query(
        "SELECT id FROM gateway_invocations
          WHERE id = $1 FOR UPDATE",
    )
    .bind(invocations[0])
    .execute(&mut *lock)
    .await
    .expect("lock stale invocation");
    assert_eq!(
        authority
            .recover_abandoned_service_invocations(now)
            .await
            .expect("recover first bounded batch"),
        128
    );
    lock.rollback().await.expect("release invocation lock");
    assert_eq!(
        authority
            .recover_abandoned_service_invocations(now)
            .await
            .expect("recover locked invocation"),
        1
    );
    let timed_out: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_invocations
          WHERE id = ANY($1) AND outcome = 'timed_out'",
    )
    .bind(&invocations)
    .fetch_one(&pool)
    .await
    .expect("count recovered invocations");
    assert_eq!(timed_out, 129);
}

const fn authority(pool: sqlx::PgPool) -> PostgresGatewayEdgeAuthority {
    PostgresGatewayEdgeAuthority::new(
        pool,
        GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 256,
            execution_timeout: StdDuration::from_secs(10),
        },
    )
}

async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest migration");
    let max_version = max_version.expect("migrations are present");
    assert!(max_version >= 72);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

async fn seed_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'recovery owner')")
        .bind(owner)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("recovery-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("recovery-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("recovery-{repository}"))
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
    .bind(format!("recovery-{gateway}"))
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway");
    let service = contract == "http.service.v1";
    let columns = if service {
        "18080, '/ready', '/health'"
    } else {
        "NULL, NULL, NULL"
    };
    let slots = if service { "'{hook}'" } else { "'{}'" };
    let revision_sql = format!(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, 'public', '{{}}', {slots}, {columns}, $6, $7)"
    );
    sqlx::query(&revision_sql)
        .bind(revision)
        .bind(gateway)
        .bind(project)
        .bind(repository)
        .bind(contract)
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
    Fixture {
        owner,
        organization,
        project,
        gateway,
        revision,
        route,
    }
}

async fn insert_invocation(
    pool: &sqlx::PgPool,
    fixture: Fixture,
    accepted_at: OffsetDateTime,
) -> Uuid {
    let invocation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id, project_id,
             request_id, outcome, accepted_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7)",
    )
    .bind(invocation)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.route)
    .bind(fixture.project)
    .bind(Uuid::new_v4())
    .bind(accepted_at)
    .execute(pool)
    .await
    .expect("invocation");
    invocation
}

async fn insert_host_session(
    pool: &sqlx::PgPool,
    fixture: Fixture,
    invocation: Uuid,
    now: OffsetDateTime,
    expires_at: OffsetDateTime,
) -> Uuid {
    let snapshot = Uuid::new_v4();
    let session = Uuid::new_v4();
    let hash = [7_u8; 32];
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
    .execute(pool)
    .await
    .expect("snapshot");
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
    .bind(now - Duration::minutes(10))
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("host session");
    session
}

#[allow(clippy::too_many_lines)]
async fn insert_lease(
    pool: &sqlx::PgPool,
    fixture: Fixture,
    invocation: Uuid,
    session: Uuid,
) -> Uuid {
    let secret = Uuid::new_v4();
    let version = Uuid::new_v4();
    let grant = Uuid::new_v4();
    let import = Uuid::new_v4();
    let binding = Uuid::new_v4();
    let rule = Uuid::new_v4();
    let lease = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO secrets
            (id, owner_organization_id, project_id, name, status,
             allowed_delivery_modes, created_by)
         VALUES ($1, $2, $3, $4, 'active', ARRAY['brokered'], $5)",
    )
    .bind(secret)
    .bind(fixture.organization)
    .bind(fixture.project)
    .bind(format!("recovery_{:032x}", secret.as_u128()))
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("secret");
    sqlx::query(
        "INSERT INTO secret_versions
            (id, secret_id, sequence, status, algorithm, key_reference,
             data_nonce, ciphertext, wrap_nonce, wrapped_data_key,
             associated_data_hash, content_length, created_by)
         VALUES ($1, $2, 1, 'active', 'AES-256-GCM+AES-256-GCM-KW/v1',
                 'test/key', decode(repeat('01', 12), 'hex'), decode('01', 'hex'),
                 decode(repeat('02', 12), 'hex'), decode('03', 'hex'),
                 decode(repeat('04', 32), 'hex'), 1, $3)",
    )
    .bind(version)
    .bind(secret)
    .bind(fixture.owner)
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
    .bind(fixture.organization)
    .bind(fixture.project)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("grant");
    sqlx::query(
        "INSERT INTO secret_imports
            (id, grant_id, secret_id, target_kind, target_id, alias, status,
             accepted_by)
         VALUES ($1, $2, $3, 'project', $4, $5, 'active', $6)",
    )
    .bind(import)
    .bind(grant)
    .bind(secret)
    .bind(fixture.project)
    .bind(format!("recovery_{:032x}", import.as_u128()))
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("import");
    sqlx::query(
        "INSERT INTO gateway_secret_bindings
            (id, gateway_id, gateway_revision_id, import_id, slot_key,
             secret_version_id, status, normalized_hash)
         VALUES ($1, $2, $3, $4, 'hook', $5, 'active', $6)",
    )
    .bind(binding)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(import)
    .bind(version)
    .bind([5_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("binding");
    sqlx::query(
        "INSERT INTO gateway_brokered_secret_rules
            (id, binding_id, gateway_revision_id, gateway_route_id,
             header_name, normalized_hash)
         VALUES ($1, $2, $3, $4, 'x-hook-secret', $5)",
    )
    .bind(rule)
    .bind(binding)
    .bind(fixture.revision)
    .bind(fixture.route)
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("rule");
    sqlx::query(
        "INSERT INTO gateway_secret_leases
            (id, runtime_session_id, invocation_id, binding_id,
             secret_version_id, rule_id, status, expires_at)
         SELECT $1, $2, $3, $4, $5, $6, 'active', session.expires_at
           FROM gateway_runtime_authority_sessions AS session
          WHERE session.id = $2",
    )
    .bind(lease)
    .bind(session)
    .bind(invocation)
    .bind(binding)
    .bind(version)
    .bind(rule)
    .execute(pool)
    .await
    .expect("lease");
    lease
}

async fn outcome(pool: &sqlx::PgPool, invocation: Uuid) -> String {
    sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .fetch_one(pool)
        .await
        .expect("outcome")
}

async fn session_status(pool: &sqlx::PgPool, session: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
        .bind(session)
        .fetch_one(pool)
        .await
        .expect("session status")
}

async fn lease_status(pool: &sqlx::PgPool, lease: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE id = $1")
        .bind(lease)
        .fetch_one(pool)
        .await
        .expect("lease status")
}

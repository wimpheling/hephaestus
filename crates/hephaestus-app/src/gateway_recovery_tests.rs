//! Real `PostgreSQL` proof for daemon-owned service invocation recovery.

use super::gateway_reconciliation_loop;
use async_trait::async_trait;
use bytes::Bytes;
use gateway_edge::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayLimits,
    GatewayProvider, GatewayProviderResponse, GatewayRequest, GatewayResponse,
};
use gateway_postgres::PostgresGatewayEdgeAuthority;
use http::{HeaderMap, StatusCode};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration as StdDuration,
};
use time::{Duration, OffsetDateTime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Copy)]
struct Fixture {
    owner: Uuid,
    organization: Uuid,
    project: Uuid,
    gateway: Uuid,
    revision: Uuid,
    route: Uuid,
    service_instance: Option<Uuid>,
}

struct RecoveryProvider {
    reconciles: Arc<AtomicUsize>,
}

impl RecoveryProvider {
    fn new(reconciles: Arc<AtomicUsize>) -> Self {
        Self { reconciles }
    }
}

#[async_trait]
impl GatewayProvider for RecoveryProvider {
    async fn reconcile(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        self.reconciles.fetch_add(1, Ordering::Release);
        Ok(desired.revision)
    }

    async fn forward(&self, _request: GatewayRequest) -> GatewayProviderResponse {
        GatewayProviderResponse {
            response: GatewayResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                headers: HeaderMap::new(),
                body: Bytes::new(),
                mailbox_publication: None,
            },
            invocation_id: Uuid::new_v4(),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_reconciliation_reaps_abandoned_service_invocation_and_joins() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let invocation = insert_invocation(&pool, fixture, now).await;
    let session =
        insert_host_session(&pool, fixture, invocation, now, now + Duration::minutes(10)).await;
    let lease = insert_lease(&pool, fixture, invocation, session).await;
    let live = seed_fixture(&pool, "http.service.v1").await;
    let live_invocation = insert_invocation(&pool, live, now).await;
    let live_session = insert_host_session(
        &pool,
        live,
        live_invocation,
        now,
        now + Duration::minutes(10),
    )
    .await;
    let live_lease = insert_lease(&pool, live, live_invocation, live_session).await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'stopping'
          WHERE id = $1",
    )
    .bind(fixture.service_instance)
    .execute(&pool)
    .await
    .expect("mark service instance stopping");

    let reconciles = Arc::new(AtomicUsize::new(0));
    let authority = make_authority(pool.clone());
    let recovery_authority = make_authority(worker_pool().await);
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(gateway_reconciliation_loop(
        authority,
        recovery_authority,
        Arc::new(RecoveryProvider::new(Arc::clone(&reconciles))),
        cancellation.clone(),
    ));
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let outcome: String =
                sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
                    .bind(invocation)
                    .fetch_one(&pool)
                    .await
                    .expect("read invocation outcome");
            if outcome == "timed_out" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("daemon recovery reaches the abandoned invocation");
    let session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .expect("read runtime session status");
    assert_eq!(session_status, "revoked");
    let lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE id = $1")
            .bind(lease)
            .fetch_one(&pool)
            .await
            .expect("read secret lease status");
    assert_eq!(lease_status, "revoked");
    let live_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(live.service_instance)
            .fetch_one(&pool)
            .await
            .expect("read live service state");
    assert_eq!(live_state, "ready");
    let live_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(live_invocation)
            .fetch_one(&pool)
            .await
            .expect("read live invocation outcome");
    assert_eq!(live_outcome, "accepted");
    let live_session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(live_session)
            .fetch_one(&pool)
            .await
            .expect("read live session status");
    assert_eq!(live_session_status, "active");
    let live_lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE id = $1")
            .bind(live_lease)
            .fetch_one(&pool)
            .await
            .expect("read live lease status");
    assert_eq!(live_lease_status, "active");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .expect("reconciliation loop joins on shutdown")
        .expect("reconciliation task join");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_recovery_cancellation_joins_a_database_blocked_batch() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let invocation = insert_invocation(&pool, fixture, now).await;
    let session =
        insert_host_session(&pool, fixture, invocation, now, now + Duration::minutes(10)).await;
    let _lease = insert_lease(&pool, fixture, invocation, session).await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'stopping'
          WHERE id = $1",
    )
    .bind(fixture.service_instance)
    .execute(&pool)
    .await
    .expect("mark service instance stopping");

    let mut lock = pool.begin().await.expect("begin authority lock");
    sqlx::query(
        "SELECT id
           FROM gateway_runtime_authority_sessions
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(session)
    .execute(&mut *lock)
    .await
    .expect("lock runtime authority session");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *lock)
        .await
        .expect("read lock holder backend pid");

    let reconciles = Arc::new(AtomicUsize::new(0));
    let authority = make_authority(pool.clone());
    let recovery_authority = make_authority(worker_pool().await);
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(gateway_reconciliation_loop(
        authority,
        recovery_authority,
        Arc::new(RecoveryProvider::new(Arc::clone(&reconciles))),
        cancellation.clone(),
    ));
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let waiters: Vec<(i32, String)> = sqlx::query_as(
                "SELECT pid, application_name
                   FROM pg_stat_activity
                  WHERE wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(holder_pid)
            .fetch_all(&pool)
            .await
            .expect("inspect PostgreSQL lock wait");
            if waiters
                .iter()
                .any(|(_, application_name)| application_name == "gateway-recovery-test")
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("recovery batch waits on the held authority row");
    let reconciles_before = reconciles.load(Ordering::Acquire);
    tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let waiters: Vec<(i32, String)> = sqlx::query_as(
                "SELECT pid, application_name
                   FROM pg_stat_activity
                  WHERE wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(holder_pid)
            .fetch_all(&pool)
            .await
            .expect("inspect PostgreSQL lock wait");
            let worker_waiting = waiters
                .iter()
                .any(|(_, application_name)| application_name == "gateway-recovery-test");
            if worker_waiting && reconciles.load(Ordering::Acquire) > reconciles_before {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("Caddy reconciliation advances while recovery waits");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .expect("blocked recovery loop joins on cancellation")
        .expect("reconciliation task join");
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted invocation outcome");
    assert_eq!(outcome, "accepted");
    let session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted session status");
    assert_eq!(session_status, "active");
    let lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE invocation_id = $1")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted lease status");
    assert_eq!(lease_status, "active");
    lock.rollback().await.expect("release authority lock");
}

const fn make_authority(pool: sqlx::PgPool) -> PostgresGatewayEdgeAuthority {
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
    assert!(max_version >= 76);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

async fn worker_pool() -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-recovery-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker PostgreSQL pool");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("read worker current user");
    assert_eq!(current_user, "hephaestus_worker");
    pool
}

// This real-PostgreSQL fixture deliberately builds the release, agent,
// revision, route, and leased instance graph in one place.
async fn seed_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    seed_fixture_with_lease(pool, contract, false).await
}

#[allow(clippy::too_many_lines)]
async fn seed_fixture_with_lease(
    pool: &sqlx::PgPool,
    contract: &str,
    expired_instance: bool,
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
        .bind(format!("recovery-{}", release.simple()))
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
        .bind(format!("recovery-agent-{}", release.simple()))
        .execute(pool)
        .await
        .expect("agent family");
        sqlx::query(
            "INSERT INTO release_agents
                (id, release_id, family_id, agent_key, display_name,
                 runtime_contract, runtime_contract_hash, parameter_schema,
                 secret_slot_schema, requires_state)
             VALUES ($1, $2, $3, 'recovery-service', 'Recovery service',
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
    let columns = if service {
        "18080, '/ready', '/health'"
    } else {
        "NULL, NULL, NULL"
    };
    let slots = if service { "'{hook}'" } else { "'{}'" };
    let revision_sql = format!(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5,
                 $6, $7, $8, 'public', '{{}}', {slots}, {columns}, $9, $10)"
    );
    sqlx::query(&revision_sql)
        .bind(revision)
        .bind(gateway)
        .bind(project)
        .bind(repository)
        .bind(if service { Some(release) } else { None })
        .bind(if service { Some(release_agent) } else { None })
        .bind(if service {
            Some("recovery-service")
        } else {
            None
        })
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
    .bind(format!("/service-{}", contract.replace('.', "-")))
    .execute(pool)
    .await
    .expect("route");
    let service_instance = if service {
        let instance = Uuid::new_v4();
        let (heartbeat, lease) = if expired_instance {
            (
                "now() - interval '20 minutes'",
                "now() - interval '10 minutes'",
            )
        } else {
            ("now()", "now() + interval '10 minutes'")
        };
        sqlx::query(&format!(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, 'recovery-host', $4, 1,
                     $5, 'ready', {lease}, {heartbeat})"
        ))
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
    Fixture {
        owner,
        organization,
        project,
        gateway,
        revision,
        route,
        service_instance,
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
             request_id, outcome, accepted_at,
             service_instance_id, service_instance_fencing_token)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, $8, $9)",
    )
    .bind(invocation)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.route)
    .bind(fixture.project)
    .bind(Uuid::new_v4())
    .bind(accepted_at)
    .bind(fixture.service_instance)
    .bind(fixture.service_instance.map(|_| 1_i64))
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

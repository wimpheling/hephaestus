//! Real `PostgreSQL` coverage for gateway admission-mode selection and cleanup.

use async_trait::async_trait;
use capability_domain::{
    AuthorityHash, RuntimeCredentialGeneration, RuntimeSessionId, RuntimeSessionStatus,
};
use gateway_edge::{GatewayInvocationRecorder, GatewayLimits, GatewayRouteBinding};
use gateway_postgres::PostgresGatewayEdgeAuthority;
use http::Method;
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, RuntimeAuthorityError,
    StoredRuntimeSession,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{
    collections::BTreeSet,
    env,
    sync::{Arc, Mutex},
    time::Duration,
};
use time::OffsetDateTime;
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Clone)]
struct RecordingIssuer {
    pool: sqlx::PgPool,
    calls: Arc<Mutex<Vec<&'static str>>>,
    entered: Option<Arc<Notify>>,
    release: Option<Arc<Notify>>,
}

struct FailingIssuer;

struct GhostIssuer;

#[derive(Clone, Copy)]
struct Fixture {
    revision: Uuid,
    route: Uuid,
}

#[async_trait]
impl GatewayRuntimeAuthorityIssuer for RecordingIssuer {
    async fn issue_gateway(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        self.calls
            .lock()
            .expect("recording issuer mutex")
            .push("guest");
        let session = persist_recorded_session(&self.pool, request, false).await?;
        self.pause_if_requested().await;
        Ok(session)
    }

    async fn issue_gateway_service(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        self.calls
            .lock()
            .expect("recording issuer mutex")
            .push("service");
        let session = persist_recorded_session(&self.pool, request, true).await?;
        self.pause_if_requested().await;
        Ok(session)
    }
}

impl RecordingIssuer {
    async fn pause_if_requested(&self) {
        if let (Some(entered), Some(release)) = (&self.entered, &self.release) {
            entered.notify_one();
            release.notified().await;
        }
    }
}

#[async_trait]
impl GatewayRuntimeAuthorityIssuer for FailingIssuer {
    async fn issue_gateway(
        &self,
        _request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        Err(RuntimeAuthorityError::Persistence)
    }
}

#[async_trait]
impl GatewayRuntimeAuthorityIssuer for GhostIssuer {
    async fn issue_gateway(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        Ok(ghost_session(request))
    }

    async fn issue_gateway_service(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        Ok(ghost_session(request))
    }
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_acceptance_selects_service_and_guest_modes() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let issuer = Arc::new(RecordingIssuer {
        pool: pool.clone(),
        calls: Arc::clone(&calls),
        entered: None,
        release: None,
    });
    let authority = build_authority(pool.clone(), issuer);
    let service = seed_fixture(&pool, "http.service.v1").await;
    let service_invocation = authority
        .accepted(&route(&service, "service"), Uuid::new_v4())
        .await
        .expect("service acceptance");
    let guest = seed_fixture(&pool, "http.v1").await;
    let guest_invocation = authority
        .accepted(&route(&guest, "guest"), Uuid::new_v4())
        .await
        .expect("guest acceptance");

    assert_eq!(
        &*calls.lock().expect("recording issuer mutex"),
        &["service", "guest"]
    );
    let service_shape: (String, String, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT session.admission_mode, session.status, session.credential_hash
           FROM gateway_runtime_authority_sessions AS session
          WHERE session.invocation_id = $1",
    )
    .bind(service_invocation)
    .fetch_one(&pool)
    .await
    .expect("service session shape");
    assert_eq!(
        service_shape,
        ("host_mediated".into(), "active".into(), None)
    );
    let guest_shape: (String, String, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT session.admission_mode, session.status, session.credential_hash
           FROM gateway_runtime_authority_sessions AS session
          WHERE session.invocation_id = $1",
    )
    .bind(guest_invocation)
    .fetch_one(&pool)
    .await
    .expect("guest session shape");
    assert_eq!(guest_shape.0, "guest_handoff");
    assert_eq!(guest_shape.1, "pending_handoff");
    assert_eq!(guest_shape.2.as_ref().map(Vec::len), Some(32));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_service_requires_runtime_issuer_but_stateless_does_not() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let authority = PostgresGatewayEdgeAuthority::new(pool.clone(), limits());
    let service = seed_fixture(&pool, "http.service.v1").await;
    assert!(
        authority
            .accepted(&route(&service, "missing-issuer"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(
        invocation_outcome(&pool, service.route).await,
        Some("rejected")
    );

    let stateless = seed_fixture(&pool, "http.v1").await;
    let invocation = authority
        .accepted(&route(&stateless, "no-issuer"), Uuid::new_v4())
        .await
        .expect("stateless acceptance without runtime issuer");
    assert_eq!(
        invocation_outcome(&pool, stateless.route).await,
        Some("accepted")
    );
    assert_eq!(session_count(&pool, invocation).await, 0);

    let overflow = seed_fixture(&pool, "http.service.v1").await;
    let overflow_authority = PostgresGatewayEdgeAuthority::new(pool.clone(), limits())
        .with_runtime_authority(
            Arc::new(RecordingIssuer {
                pool: pool.clone(),
                calls: Arc::new(Mutex::new(Vec::new())),
                entered: None,
                release: None,
            }),
            Duration::from_secs(u64::MAX),
        )
        .expect("nonzero oversized TTL is accepted as configuration");
    assert!(
        overflow_authority
            .accepted(&route(&overflow, "ttl-overflow"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(
        invocation_outcome(&pool, overflow.route).await,
        Some("rejected")
    );

    let date_overflow = seed_fixture(&pool, "http.service.v1").await;
    let date_overflow_authority = PostgresGatewayEdgeAuthority::new(pool.clone(), limits())
        .with_runtime_authority(
            Arc::new(RecordingIssuer {
                pool: pool.clone(),
                calls: Arc::new(Mutex::new(Vec::new())),
                entered: None,
                release: None,
            }),
            Duration::from_secs(1_000_000_000_000),
        )
        .expect("large representable TTL is accepted as configuration");
    assert!(
        date_overflow_authority
            .accepted(&route(&date_overflow, "date-overflow"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(
        invocation_outcome(&pool, date_overflow.route).await,
        Some("rejected")
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_acceptance_failures_terminally_reject_invocations() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let failed = seed_fixture(&pool, "http.service.v1").await;
    let authority = build_authority(pool.clone(), Arc::new(FailingIssuer));
    assert!(
        authority
            .accepted(&route(&failed, "failed"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(
        invocation_outcome(&pool, failed.route).await,
        Some("rejected")
    );

    let ghost = seed_fixture(&pool, "http.service.v1").await;
    let authority = build_authority(pool.clone(), Arc::new(GhostIssuer));
    assert!(
        authority
            .accepted(&route(&ghost, "ghost"), Uuid::new_v4())
            .await
            .is_err()
    );
    let invocation = latest_invocation(&pool, ghost.route).await;
    assert_eq!(
        invocation_outcome(&pool, ghost.route).await,
        Some("rejected")
    );
    assert_eq!(session_count(&pool, invocation).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_acceptance_completion_race_leaves_no_active_session_or_lease() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let authority = Arc::new(build_authority(
        pool.clone(),
        Arc::new(RecordingIssuer {
            pool: pool.clone(),
            calls: Arc::new(Mutex::new(Vec::new())),
            entered: Some(Arc::clone(&entered)),
            release: Some(Arc::clone(&release)),
        }),
    ));
    let accepted_route = route(&fixture, "race");
    let accepted_authority = Arc::clone(&authority);
    let accepted_task = tokio::spawn(async move {
        accepted_authority
            .accepted(&accepted_route, Uuid::new_v4())
            .await
    });
    entered.notified().await;
    let invocation = latest_invocation(&pool, fixture.route).await;
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, 'timed_out')")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .expect("terminal cleanup query")
    );
    release.notify_one();
    assert!(accepted_task.await.expect("acceptance task join").is_err());
    assert_ne!(
        session_status(&pool, invocation).await.as_deref(),
        Some("active")
    );
    assert_eq!(active_lease_count(&pool, invocation).await, 0);
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
    let version: i64 =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE version = 72")
            .fetch_one(&pool)
            .await
            .expect("migration 72");
    assert_eq!(version, 72);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 migration=72");
    Some(pool)
}

fn build_authority(
    pool: sqlx::PgPool,
    issuer: Arc<dyn GatewayRuntimeAuthorityIssuer>,
) -> PostgresGatewayEdgeAuthority {
    PostgresGatewayEdgeAuthority::new(pool, limits())
        .with_runtime_authority(issuer, Duration::from_secs(300))
        .expect("valid session TTL")
}

const fn limits() -> GatewayLimits {
    GatewayLimits {
        max_request_body_bytes: 1024,
        max_response_body_bytes: 1024,
        max_request_headers: 16,
        max_response_headers: 16,
        max_path_and_query_bytes: 256,
        execution_timeout: Duration::from_secs(10),
    }
}

fn route(fixture: &Fixture, prefix: &str) -> GatewayRouteBinding {
    GatewayRouteBinding {
        route_id: fixture.route,
        gateway_revision_id: fixture.revision,
        path_prefix: prefix.to_owned(),
        methods: BTreeSet::from([Method::GET]),
        limits: limits(),
    }
}

async fn persist_recorded_session(
    pool: &sqlx::PgPool,
    request: GatewayRuntimeSessionRequest,
    service: bool,
) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
    let hash = [7_u8; 32];
    let snapshot = request.invocation_id.as_uuid();
    let mode = if service {
        "host_mediated"
    } else {
        "guest_handoff"
    };
    let status = if service { "active" } else { "pending_handoff" };
    let credential_hash = if service { None } else { Some(hash.as_slice()) };
    sqlx::query(
        "INSERT INTO gateway_authorization_snapshots
            (id, invocation_id, gateway_id, gateway_revision_id,
             authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'test/v1', $5)",
    )
    .bind(snapshot)
    .bind(request.invocation_id.as_uuid())
    .bind(request.gateway_id)
    .bind(request.gateway_revision_id)
    .bind(hash.as_slice())
    .execute(pool)
    .await
    .map_err(|_| RuntimeAuthorityError::Persistence)?;
    sqlx::query(
        "INSERT INTO gateway_runtime_authority_sessions
            (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
             identity_hash, snapshot_hash, issuance_generation, credential_hash,
             admission_mode, status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8, $9, $10, $11, $12, $13)",
    )
    .bind(request.invocation_id.as_uuid())
    .bind(snapshot)
    .bind(request.invocation_id.as_uuid())
    .bind(request.gateway_id)
    .bind(request.gateway_revision_id)
    .bind(hash.as_slice())
    .bind(hash.as_slice())
    .bind(credential_hash)
    .bind(mode)
    .bind(status)
    .bind(request.issued_at)
    .bind(request.expires_at)
    .bind(None::<OffsetDateTime>)
    .execute(pool)
    .await
    .map_err(|_| RuntimeAuthorityError::Persistence)?;
    Ok(StoredRuntimeSession {
        id: RuntimeSessionId::from_uuid(request.invocation_id.as_uuid()),
        snapshot_id: capability_domain::AuthorizationSnapshotId::from_uuid(snapshot),
        identity_hash: AuthorityHash::from_bytes(hash),
        generation: RuntimeCredentialGeneration::INITIAL,
        status: if service {
            RuntimeSessionStatus::Active
        } else {
            RuntimeSessionStatus::PendingHandoff
        },
        issued_at: request.issued_at,
        expires_at: request.expires_at,
        acknowledged_at: None,
        revoked_at: None,
    })
}

fn ghost_session(request: GatewayRuntimeSessionRequest) -> StoredRuntimeSession {
    StoredRuntimeSession {
        id: RuntimeSessionId::from_uuid(Uuid::new_v4()),
        snapshot_id: capability_domain::AuthorizationSnapshotId::from_uuid(Uuid::new_v4()),
        identity_hash: AuthorityHash::from_bytes([3_u8; 32]),
        generation: RuntimeCredentialGeneration::INITIAL,
        status: RuntimeSessionStatus::Active,
        issued_at: request.issued_at,
        expires_at: request.expires_at,
        acknowledged_at: None,
        revoked_at: None,
    }
}

async fn seed_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
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
    let columns = if service {
        "18080, '/ready', '/health'"
    } else {
        "NULL, NULL, NULL"
    };
    let revision_sql = format!(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, 'public', '{{}}', '{{}}', {columns}, $6, $7)"
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
    Fixture { revision, route }
}

async fn latest_invocation(pool: &sqlx::PgPool, route: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM gateway_invocations
          WHERE gateway_route_id = $1 ORDER BY accepted_at DESC, id DESC LIMIT 1",
    )
    .bind(route)
    .fetch_one(pool)
    .await
    .expect("latest invocation")
}

async fn invocation_outcome(pool: &sqlx::PgPool, route: Uuid) -> Option<&'static str> {
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

async fn session_count(pool: &sqlx::PgPool, invocation: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM gateway_runtime_authority_sessions WHERE invocation_id = $1",
    )
    .bind(invocation)
    .fetch_one(pool)
    .await
    .expect("session count")
}

async fn session_status(pool: &sqlx::PgPool, invocation: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT status FROM gateway_runtime_authority_sessions WHERE invocation_id = $1",
    )
    .bind(invocation)
    .fetch_optional(pool)
    .await
    .expect("session status")
}

async fn active_lease_count(pool: &sqlx::PgPool, invocation: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM gateway_secret_leases
          WHERE invocation_id = $1 AND status = 'active'",
    )
    .bind(invocation)
    .fetch_one(pool)
    .await
    .expect("active lease count")
}

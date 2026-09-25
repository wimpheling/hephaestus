//! Real `PostgreSQL` coverage for gateway admission-mode selection and cleanup.

use async_trait::async_trait;
use capability_domain::{
    AuthorityHash, RuntimeCredentialGeneration, RuntimeSessionId, RuntimeSessionStatus,
};
use gateway_domain::{
    GatewayInvocationRecorder, GatewayLimits, GatewayRouteBinding, GatewayRouteResolver,
};
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

struct TestPools {
    admin: sqlx::PgPool,
    worker: sqlx::PgPool,
}

type ActivityRow = (i32, String, String, String, Option<String>, Option<String>);

#[derive(Clone, Copy)]
struct Fixture {
    gateway: Uuid,
    revision: Uuid,
    route: Uuid,
    service_instance: Option<Uuid>,
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
    let Some(pools) = test_pool().await else {
        return;
    };
    let admin = &pools.admin;
    let worker = &pools.worker;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let issuer = Arc::new(RecordingIssuer {
        pool: worker.clone(),
        calls: Arc::clone(&calls),
        entered: None,
        release: None,
    });
    let authority = build_authority(worker.clone(), issuer);
    let service = seed_fixture(admin, "http.service.v1").await;
    let service_invocation = authority
        .accepted(&route(&service, "service"), Uuid::new_v4())
        .await
        .expect("service acceptance");
    let guest = seed_fixture(admin, "http.v1").await;
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
    .fetch_one(admin)
    .await
    .expect("service session shape");
    assert_eq!(
        service_shape,
        ("host_mediated".into(), "active".into(), None)
    );
    let service_instance = service.service_instance.expect("service instance fixture");
    let binding: (Uuid, i64) = sqlx::query_as(
        "SELECT service_instance_id, service_instance_fencing_token
           FROM gateway_invocations
          WHERE id = $1",
    )
    .bind(service_invocation)
    .fetch_one(admin)
    .await
    .expect("service invocation binding");
    assert_eq!(binding, (service_instance, 1));
    let other = seed_fixture(admin, "http.service.v1").await;
    let cross_revision = sqlx::query(
        "UPDATE gateway_invocations
            SET service_instance_id = $2,
                service_instance_fencing_token = 1
          WHERE id = $1",
    )
    .bind(service_invocation)
    .bind(other.service_instance.expect("other service instance"))
    .execute(admin)
    .await;
    assert!(cross_revision.is_err());
    assert!(
        insert_service_invocation(admin, &service, None, None)
            .await
            .is_err()
    );
    assert!(
        insert_service_invocation(admin, &service, Some(service_instance), Some(99))
            .await
            .is_err()
    );
    let guest_shape: (String, String, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT session.admission_mode, session.status, session.credential_hash
           FROM gateway_runtime_authority_sessions AS session
          WHERE session.invocation_id = $1",
    )
    .bind(guest_invocation)
    .fetch_one(admin)
    .await
    .expect("guest session shape");
    assert_eq!(guest_shape.0, "guest_handoff");
    assert_eq!(guest_shape.1, "pending_handoff");
    assert_eq!(guest_shape.2.as_ref().map(Vec::len), Some(32));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_desired_configuration_cancellation_closes_worker_backend() {
    let Some(pools) = test_pool().await else {
        return;
    };
    let admin = &pools.admin;
    let worker = &pools.worker;
    let fixture = seed_fixture(admin, "http.service.v1").await;
    let authority = build_authority(worker.clone(), Arc::new(GhostIssuer));
    let first = authority
        .desired_configuration()
        .await
        .expect("initial desired configuration");
    let second = authority
        .desired_configuration()
        .await
        .expect("healthy desired configuration reuse");
    assert_eq!(first.revision, second.revision);
    assert!(
        first
            .routes
            .iter()
            .any(|route| route.route_id == fixture.route)
    );

    let mut gateway_lock = admin.begin().await.expect("gateway lock transaction");
    sqlx::query("LOCK TABLE gateways IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *gateway_lock)
        .await
        .expect("hold gateway table lock");
    let canceled = tokio::spawn(async move { authority.desired_configuration().await });
    let waiting = wait_for_gateway_lock(admin).await;
    println!(
        "REAL_GATEWAY_ACTIVE_ROUTES_CANCELLATION_BARRIER=1 pid={} application={} state={} backend={} wait_event={}",
        waiting.0,
        waiting.1,
        waiting.2,
        waiting.3,
        waiting.5.as_deref().unwrap_or("unknown")
    );
    canceled.abort();
    assert!(
        canceled
            .await
            .expect_err("canceled desired configuration task")
            .is_cancelled(),
        "the actual adapter query must be canceled while PostgreSQL holds the table lock"
    );
    gateway_lock
        .rollback()
        .await
        .expect("release gateway table lock");

    tokio::time::timeout(Duration::from_secs(10), worker.close())
        .await
        .expect("worker pool close");
    println!(
        "REAL_GATEWAY_ACTIVE_ROUTES_CANCELLATION_POOL size={} idle={} closed={}",
        worker.size(),
        worker.num_idle(),
        worker.is_closed()
    );
    wait_for_no_gateway_acceptance_sessions(admin).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn heph_authenticated_revision_is_excluded_from_public_admission() {
    let Some(pools) = test_pool().await else {
        return;
    };
    let fixture =
        seed_fixture_with_exposure(&pools.admin, "http.service.v1", "heph_authenticated").await;
    let public_fixture = seed_fixture(&pools.admin, "http.service.v1").await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let issuer = Arc::new(RecordingIssuer {
        pool: pools.worker.clone(),
        calls: Arc::clone(&calls),
        entered: None,
        release: None,
    });

    let authority = build_authority(pools.worker.clone(), issuer);
    let public_invocation = authority
        .accepted(&route(&public_fixture, "public-control"), Uuid::new_v4())
        .await
        .expect("public control admission");
    assert_eq!(
        session_status(&pools.admin, public_invocation)
            .await
            .as_deref(),
        Some("active")
    );
    assert_eq!(
        &*calls.lock().expect("recording issuer mutex"),
        &["service"]
    );
    let desired = authority
        .desired_configuration()
        .await
        .expect("public desired configuration");
    assert!(
        !desired
            .routes
            .iter()
            .any(|route| route.route_id == fixture.route)
    );
    let resolved = authority
        .resolve("/gateway/http.service.v1")
        .await
        .expect("public route resolution")
        .map(|route| route.route_id);
    assert_ne!(
        resolved,
        Some(fixture.route),
        "authenticated route must not be publicly resolved"
    );
    let calls_before = calls.lock().expect("recording issuer mutex").len();
    let authenticated_invocations_before = invocation_count(&pools.admin, fixture.route).await;
    assert!(
        authority
            .accepted(&route(&fixture, "authenticated-control"), Uuid::new_v4())
            .await
            .is_err(),
        "public admission must recheck the authoritative exposure"
    );
    assert_eq!(
        calls.lock().expect("recording issuer mutex").len(),
        calls_before,
        "reserved exposure must fail before issuing a runtime session"
    );
    assert_eq!(
        invocation_count(&pools.admin, fixture.route).await,
        authenticated_invocations_before,
        "reserved exposure must fail before inserting an invocation"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_acceptance_rechecks_route_after_waiting_for_gateway_lock() {
    let Some(pools) = test_pool().await else {
        return;
    };
    let admin = &pools.admin;
    let worker = &pools.worker;
    let fixture = seed_fixture(admin, "http.service.v1").await;
    let mut lock = admin.begin().await.expect("gateway lock transaction");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .execute(&mut *lock)
        .await
        .expect("hold gateway lock");

    let calls = Arc::new(Mutex::new(Vec::new()));
    let authority = build_authority(
        worker.clone(),
        Arc::new(RecordingIssuer {
            pool: worker.clone(),
            calls: Arc::clone(&calls),
            entered: None,
            release: None,
        }),
    );
    let accepted = tokio::spawn(async move {
        authority
            .accepted(&route(&fixture, "locked"), Uuid::new_v4())
            .await
    });
    wait_for_gateway_lock(admin).await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = NULL
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .execute(&mut *lock)
    .await
    .expect("switch active revision while holding lock");
    lock.commit().await.expect("commit active switch");
    assert!(accepted.await.expect("acceptance join").is_err());
    assert_eq!(invocation_count(admin, fixture.route).await, 0);
    assert!(calls.lock().expect("recording issuer mutex").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_service_requires_runtime_issuer_but_stateless_does_not() {
    let Some(pools) = test_pool().await else {
        return;
    };
    let admin = &pools.admin;
    let worker = &pools.worker;
    let authority = PostgresGatewayEdgeAuthority::new(worker.clone(), limits());
    let service = seed_fixture(admin, "http.service.v1").await;
    assert!(
        authority
            .accepted(&route(&service, "missing-issuer"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(
        invocation_outcome(admin, service.route).await,
        Some("rejected")
    );

    let stateless = seed_fixture(admin, "http.v1").await;
    let invocation = authority
        .accepted(&route(&stateless, "no-issuer"), Uuid::new_v4())
        .await
        .expect("stateless acceptance without runtime issuer");
    assert_eq!(
        invocation_outcome(admin, stateless.route).await,
        Some("accepted")
    );
    assert_eq!(session_count(admin, invocation).await, 0);

    let overflow = seed_fixture(admin, "http.service.v1").await;
    let overflow_authority = PostgresGatewayEdgeAuthority::new(worker.clone(), limits())
        .with_runtime_authority(
            Arc::new(RecordingIssuer {
                pool: worker.clone(),
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
        invocation_outcome(admin, overflow.route).await,
        Some("rejected")
    );

    let date_overflow = seed_fixture(admin, "http.service.v1").await;
    let date_overflow_authority = PostgresGatewayEdgeAuthority::new(worker.clone(), limits())
        .with_runtime_authority(
            Arc::new(RecordingIssuer {
                pool: worker.clone(),
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
        invocation_outcome(admin, date_overflow.route).await,
        Some("rejected")
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_service_acceptance_requires_a_live_ready_instance() {
    let Some(pools) = test_pool().await else {
        return;
    };
    let admin = &pools.admin;
    let worker = &pools.worker;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let authority = build_authority(
        worker.clone(),
        Arc::new(RecordingIssuer {
            pool: worker.clone(),
            calls: Arc::clone(&calls),
            entered: None,
            release: None,
        }),
    );

    let draining = seed_fixture(admin, "http.service.v1").await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'draining'
          WHERE id = $1",
    )
    .bind(draining.service_instance.expect("draining instance"))
    .execute(admin)
    .await
    .expect("drain service instance");
    assert!(
        authority
            .accepted(&route(&draining, "draining"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(invocation_count(admin, draining.route).await, 0);

    let expired = seed_fixture_with_expiry(admin, "http.service.v1", false).await;
    assert!(
        authority
            .accepted(&route(&expired, "expired"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(invocation_count(admin, expired.route).await, 0);

    let missing = seed_fixture(admin, "http.service.v1").await;
    sqlx::query("DELETE FROM gateway_service_instances WHERE id = $1")
        .bind(missing.service_instance.expect("missing instance"))
        .execute(admin)
        .await
        .expect("remove service instance");
    assert!(
        authority
            .accepted(&route(&missing, "missing"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(invocation_count(admin, missing.route).await, 0);
    assert!(calls.lock().expect("recording issuer mutex").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_service_acceptance_rechecks_expiry_after_waiting_for_release_lock() {
    let Some(pools) = test_pool().await else {
        return;
    };
    let admin = &pools.admin;
    let worker = &pools.worker;
    let fixture = seed_fixture_with_lease(admin, "http.service.v1", 1).await;
    let release_id: Uuid =
        sqlx::query_scalar("SELECT release_id FROM gateway_revisions WHERE id = $1")
            .bind(fixture.revision)
            .fetch_one(admin)
            .await
            .expect("service release");
    let mut release_lock = admin.begin().await.expect("release lock transaction");
    sqlx::query("SELECT id FROM releases WHERE id = $1 FOR UPDATE")
        .bind(release_id)
        .execute(&mut *release_lock)
        .await
        .expect("hold release lock");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let authority = build_authority(
        worker.clone(),
        Arc::new(RecordingIssuer {
            pool: worker.clone(),
            calls: Arc::clone(&calls),
            entered: None,
            release: None,
        }),
    );
    let accepted = tokio::spawn(async move {
        authority
            .accepted(&route(&fixture, "release-locked"), Uuid::new_v4())
            .await
    });
    wait_for_gateway_lock(admin).await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    release_lock.commit().await.expect("release release lock");
    assert!(accepted.await.expect("acceptance join").is_err());
    assert_eq!(invocation_count(admin, fixture.route).await, 0);
    assert!(calls.lock().expect("recording issuer mutex").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_acceptance_failures_terminally_reject_invocations() {
    let Some(pools) = test_pool().await else {
        return;
    };
    let admin = &pools.admin;
    let worker = &pools.worker;
    let failed = seed_fixture(admin, "http.service.v1").await;
    let authority = build_authority(worker.clone(), Arc::new(FailingIssuer));
    assert!(
        authority
            .accepted(&route(&failed, "failed"), Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(
        invocation_outcome(admin, failed.route).await,
        Some("rejected")
    );

    let ghost = seed_fixture(admin, "http.service.v1").await;
    let authority = build_authority(worker.clone(), Arc::new(GhostIssuer));
    assert!(
        authority
            .accepted(&route(&ghost, "ghost"), Uuid::new_v4())
            .await
            .is_err()
    );
    let invocation = latest_invocation(admin, ghost.route).await;
    assert_eq!(
        invocation_outcome(admin, ghost.route).await,
        Some("rejected")
    );
    assert_eq!(session_count(admin, invocation).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_acceptance_completion_race_leaves_no_active_session_or_lease() {
    let Some(pools) = test_pool().await else {
        return;
    };
    let admin = &pools.admin;
    let worker = &pools.worker;
    let fixture = seed_fixture(admin, "http.service.v1").await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let authority = Arc::new(build_authority(
        worker.clone(),
        Arc::new(RecordingIssuer {
            pool: worker.clone(),
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
    let invocation = latest_invocation(admin, fixture.route).await;
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, 'timed_out')")
            .bind(invocation)
            .fetch_one(admin)
            .await
            .expect("terminal cleanup query")
    );
    release.notify_one();
    assert!(accepted_task.await.expect("acceptance task join").is_err());
    assert_ne!(
        session_status(admin, invocation).await.as_deref(),
        Some("active")
    );
    assert_eq!(active_lease_count(admin, invocation).await, 0);
}

async fn test_pool() -> Option<TestPools> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let admin = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&admin)
        .await
        .expect("apply gateway migrations");
    let version: i64 =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE version = 75")
            .fetch_one(&admin)
            .await
            .expect("migration 75");
    assert_eq!(version, 75);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 migration=75");
    let worker = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-acceptance-worker'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker PostgreSQL pool");
    Some(TestPools { admin, worker })
}

async fn wait_for_gateway_lock(
    admin: &sqlx::PgPool,
) -> (i32, String, String, String, Option<String>, Option<String>) {
    for _ in 0..200 {
        let waiting: Option<ActivityRow> = sqlx::query_as(
            "SELECT pid, coalesce(application_name, ''), coalesce(state, ''),
                        coalesce(backend_type, ''), wait_event_type, wait_event
                   FROM pg_stat_activity
                  WHERE datname = current_database()
                    AND application_name = 'gateway-acceptance-worker'
                    AND wait_event_type = 'Lock'
                    AND state = 'active'
                  ORDER BY pid
                  LIMIT 1",
        )
        .fetch_optional(admin)
        .await
        .expect("inspect acceptance lock wait");
        if let Some(waiting) = waiting {
            return waiting;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("worker acceptance did not wait on the held gateway lock");
}

async fn wait_for_no_gateway_acceptance_sessions(admin: &sqlx::PgPool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let sessions: Vec<ActivityRow> = sqlx::query_as(
            "SELECT pid, coalesce(application_name, ''), coalesce(state, ''),
                        coalesce(backend_type, ''), wait_event_type, wait_event
                   FROM pg_stat_activity
                  WHERE datname = current_database()
                    AND application_name = 'gateway-acceptance-worker'
                  ORDER BY pid",
        )
        .fetch_all(admin)
        .await
        .expect("inspect gateway acceptance sessions");
        if sessions.is_empty() {
            println!("REAL_GATEWAY_ACTIVE_ROUTES_CANCELLATION_SESSIONS=0");
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "gateway acceptance worker sessions remain after pool close: {sessions:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
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
        exposure: gateway_domain::Exposure::Public,
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
    seed_fixture_with_exposure(pool, contract, "public").await
}

async fn seed_fixture_with_exposure(
    pool: &sqlx::PgPool,
    contract: &str,
    exposure: &str,
) -> Fixture {
    seed_fixture_with_timing(pool, contract, true, 600, exposure).await
}

async fn seed_fixture_with_lease(
    pool: &sqlx::PgPool,
    contract: &str,
    lease_seconds: i64,
) -> Fixture {
    seed_fixture_with_timing(pool, contract, true, lease_seconds, "public").await
}

// This real-PostgreSQL fixture deliberately builds the release, agent,
// revision, route, and leased instance graph in one place.
#[allow(clippy::too_many_lines)]
async fn seed_fixture_with_expiry(
    pool: &sqlx::PgPool,
    contract: &str,
    lease_live: bool,
) -> Fixture {
    seed_fixture_with_timing(pool, contract, lease_live, 600, "public").await
}

// This real-PostgreSQL fixture deliberately builds the release, agent,
// revision, route, and leased instance graph in one place.
#[allow(clippy::too_many_lines)]
async fn seed_fixture_with_timing(
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

async fn insert_service_invocation(
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

async fn invocation_count(pool: &sqlx::PgPool, route: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM gateway_invocations WHERE gateway_route_id = $1")
        .bind(route)
        .fetch_one(pool)
        .await
        .expect("invocation count")
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

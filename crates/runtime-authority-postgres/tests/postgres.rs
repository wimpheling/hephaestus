//! Real `PostgreSQL` proof for host-mediated gateway runtime sessions.

use async_trait::async_trait;
use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, RuntimeCredential, RuntimeCredentialGeneration,
    RuntimeInvocation, RuntimeSessionId, RuntimeSessionIdentity, RuntimeSessionStatus,
    WorkloadKind, WorkloadPrincipal,
};
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, NewRuntimeSession,
    RuntimeAuthorityError, RuntimeHandoffStore, RuntimeSessionRepository,
};
use runtime_authority_postgres::{
    PgGatewayRuntimeAuthorityIssuer, PgGatewayRuntimeSessionRepository,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

#[derive(Clone)]
struct CountingHandoff {
    creates: Arc<AtomicUsize>,
}

#[async_trait]
impl RuntimeHandoffStore for CountingHandoff {
    fn create(
        &self,
        _session_id: RuntimeSessionId,
        _generation: RuntimeCredentialGeneration,
        _expires_at: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(RuntimeCredential::from_secret([7; 32]))
    }

    fn open(
        &self,
        _session_id: RuntimeSessionId,
        _generation: RuntimeCredentialGeneration,
        _now: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        Ok(RuntimeCredential::from_secret([7; 32]))
    }

    fn destroy(
        &self,
        _session_id: RuntimeSessionId,
        _generation: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeAuthorityError> {
        Ok(())
    }

    fn purge_expired(&self, _now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        Ok(0)
    }
}

struct Fixture {
    owner: Uuid,
    organization: Uuid,
    project: Uuid,
    gateway: Uuid,
    revision: Uuid,
    route: Uuid,
    invocation: Uuid,
    service_instance: Option<Uuid>,
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_host_mediated_sessions_are_distinct_and_lifecycle_bound() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect runtime authority PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply runtime authority migrations");
    let migration_version: i64 =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE version = 75")
            .fetch_one(&pool)
            .await
            .expect("confirm migration 0075 applied to the connected database");
    assert_eq!(migration_version, 75);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 migration=75");

    let creates = Arc::new(AtomicUsize::new(0));
    let issuer = Arc::new(PgGatewayRuntimeAuthorityIssuer::new(
        pool.clone(),
        CountingHandoff {
            creates: Arc::clone(&creates),
        },
        "test/v1",
    ));
    let repository = PgGatewayRuntimeSessionRepository::new(pool.clone());

    let service = seed_fixture(&pool, "http.service.v1").await;
    let service_request = request(&service, OffsetDateTime::now_utc(), Duration::minutes(10));
    let first = issuer
        .issue_gateway_service(service_request)
        .await
        .expect("issue host-mediated service session");
    let first_lease = seed_gateway_lease(&pool, &service).await;
    assert_eq!(first.status, RuntimeSessionStatus::Active);
    assert_eq!(first.acknowledged_at, None);
    assert_eq!(creates.load(Ordering::SeqCst), 0);

    let concurrent = seed_fixture(&pool, "http.service.v1").await;
    let concurrent_request = request(
        &concurrent,
        OffsetDateTime::now_utc(),
        Duration::minutes(10),
    );
    let left_issuer = Arc::clone(&issuer);
    let right_issuer = Arc::clone(&issuer);
    let (left, right) = tokio::join!(
        async move { left_issuer.issue_gateway_service(concurrent_request).await },
        async move { right_issuer.issue_gateway_service(concurrent_request).await },
    );
    let left = left.expect("first concurrent host-mediated issue");
    let right = right.expect("second concurrent host-mediated issue");
    assert_eq!(
        left.id, right.id,
        "concurrent retries must share one session"
    );

    let second = issuer
        .issue_gateway_service(service_request)
        .await
        .expect("retry host-mediated service session");
    assert_eq!(
        second.id, first.id,
        "request retry must reuse the exact session"
    );
    assert_eq!(second.snapshot_id, first.snapshot_id);
    assert_eq!(second.identity_hash, first.identity_hash);
    assert_eq!(second.generation, first.generation);
    assert_eq!(second.status, first.status);
    assert_eq!(second.acknowledged_at, first.acknowledged_at);
    let row: (String, Option<Vec<u8>>, String, Option<OffsetDateTime>) = sqlx::query_as(
        "SELECT admission_mode, credential_hash, status, acknowledged_at
         FROM gateway_runtime_authority_sessions WHERE id = $1",
    )
    .bind(service.invocation)
    .fetch_one(&pool)
    .await
    .expect("load host-mediated session shape");
    assert_eq!(row.0, "host_mediated");
    assert_eq!(row.1, None);
    assert_eq!(row.2, "active");
    assert_eq!(row.3, None);
    sqlx::query(
        "UPDATE gateway_runtime_authority_sessions
            SET credential_hash = NULL WHERE id = $1",
    )
    .bind(service.invocation)
    .execute(&pool)
    .await
    .expect("NULL credential verifier remains NULL-safe");
    assert!(
        sqlx::query(
            "UPDATE gateway_runtime_authority_sessions
                SET credential_hash = decode(repeat('01', 32), 'hex')
              WHERE id = $1",
        )
        .bind(service.invocation)
        .execute(&pool)
        .await
        .is_err(),
        "credential verifier mutation must be rejected"
    );
    assert!(
        sqlx::query(
            "UPDATE gateway_runtime_authority_sessions
                SET admission_mode = 'guest_handoff' WHERE id = $1",
        )
        .bind(service.invocation)
        .execute(&pool)
        .await
        .is_err(),
        "admission mode mutation must be rejected"
    );
    assert!(
        sqlx::query(
            "UPDATE gateway_runtime_authority_sessions
                SET status = 'pending_handoff' WHERE id = $1",
        )
        .bind(service.invocation)
        .execute(&pool)
        .await
        .is_err(),
        "host-mediated sessions cannot become pending guest handoffs"
    );
    assert!(
        sqlx::query(
            "UPDATE gateway_runtime_authority_sessions
                SET acknowledged_at = now() WHERE id = $1",
        )
        .bind(service.invocation)
        .execute(&pool)
        .await
        .is_err(),
        "host-mediated sessions cannot gain a guest acknowledgement"
    );
    let changed_identity_request = GatewayRuntimeSessionRequest {
        issued_at: service_request.issued_at + Duration::seconds(1),
        expires_at: service_request.expires_at + Duration::seconds(1),
        ..service_request
    };
    assert_eq!(
        issuer.issue_gateway_service(changed_identity_request).await,
        Err(RuntimeAuthorityError::IdentityMismatch),
        "changed identity retry must not reuse the session"
    );
    assert_eq!(
        repository
            .acknowledge(
                RuntimeSessionId::from_uuid(service.invocation),
                RuntimeCredentialGeneration::INITIAL,
                OffsetDateTime::now_utc(),
            )
            .await,
        Err(RuntimeAuthorityError::SessionNotPending),
        "host-mediated sessions cannot enter the guest acknowledgement path"
    );

    let revoked = repository
        .revoke(
            RuntimeSessionId::from_uuid(service.invocation),
            OffsetDateTime::now_utc(),
            "service request finished",
        )
        .await
        .expect("revoke host-mediated session");
    assert_eq!(revoked.status, RuntimeSessionStatus::Revoked);
    assert_eq!(revoked.acknowledged_at, None);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM gateway_secret_leases WHERE id = $1",)
            .bind(first_lease)
            .fetch_one(&pool)
            .await
            .expect("load revoked gateway lease"),
        "revoked"
    );
    assert_eq!(
        issuer.issue_gateway_service(service_request).await,
        Err(RuntimeAuthorityError::SessionNotPending),
        "revoked host-mediated sessions cannot be retried"
    );

    let direct_snapshot = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(service.invocation),
        WorkloadPrincipal::new(WorkloadKind::Gateway, service.gateway, service.revision),
        "test/v1",
        Vec::new(),
    )
    .expect("direct guest snapshot");
    let direct_identity = RuntimeSessionIdentity::new(
        RuntimeSessionId::from_uuid(service.invocation),
        direct_snapshot.principal(),
        RuntimeInvocation::Gateway(capability_domain::GatewayInvocationId::from_uuid(
            service.invocation,
        )),
        &direct_snapshot,
        service_request.issued_at,
        service_request.expires_at,
    )
    .expect("direct guest identity");
    let direct_credential = RuntimeCredential::from_secret([8; 32]);
    assert_eq!(
        repository
            .create(NewRuntimeSession {
                snapshot: &direct_snapshot,
                identity: &direct_identity,
                generation: RuntimeCredentialGeneration::INITIAL,
                credential_hash: direct_credential
                    .storage_hash(direct_identity.id(), RuntimeCredentialGeneration::INITIAL,),
                attachment_id: None,
            })
            .await,
        Err(RuntimeAuthorityError::IdentityMismatch),
        "direct guest repository creation must reject a service revision"
    );

    for outcome in ["completed", "failed", "timed_out"] {
        let terminal = seed_fixture(&pool, "http.service.v1").await;
        let terminal_request = request(&terminal, OffsetDateTime::now_utc(), Duration::minutes(10));
        issuer
            .issue_gateway_service(terminal_request)
            .await
            .expect("issue terminal-cleanup service session");
        seed_gateway_lease(&pool, &terminal).await;
        let changed: bool = sqlx::query_scalar("SELECT gateway_invocation_complete($1, $2)")
            .bind(terminal.invocation)
            .bind(outcome)
            .fetch_one(&pool)
            .await
            .expect("complete host-mediated gateway invocation");
        assert!(changed);
        let statuses: (String, String, String) = sqlx::query_as(
            "SELECT invocation.outcome, session.status, lease.status
               FROM gateway_invocations AS invocation
               JOIN gateway_runtime_authority_sessions AS session
                 ON session.invocation_id = invocation.id
               JOIN gateway_secret_leases AS lease
                 ON lease.invocation_id = invocation.id
              WHERE invocation.id = $1",
        )
        .bind(terminal.invocation)
        .fetch_one(&pool)
        .await
        .expect("load terminal cleanup statuses");
        assert_eq!(
            statuses,
            (
                outcome.to_owned(),
                String::from("revoked"),
                String::from("revoked")
            )
        );
        assert_eq!(
            issuer.issue_gateway_service(terminal_request).await,
            Err(RuntimeAuthorityError::SessionNotPending),
            "terminal service retries must be rejected"
        );
        assert!(
            !sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, $2)")
                .bind(terminal.invocation)
                .bind(outcome)
                .fetch_one(&pool)
                .await
                .expect("repeat terminal completion"),
            "terminal completion is idempotent"
        );
    }

    let raced = seed_fixture(&pool, "http.service.v1").await;
    let raced_request = request(&raced, OffsetDateTime::now_utc(), Duration::minutes(10));
    let race_issuer = Arc::clone(&issuer);
    let (race_issue, race_complete) = tokio::join!(
        async move { race_issuer.issue_gateway_service(raced_request).await },
        async {
            sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, 'timed_out')")
                .bind(raced.invocation)
                .fetch_one(&pool)
                .await
        },
    );
    assert!(race_complete.expect("race completion query"));
    let final_race_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM gateway_runtime_authority_sessions WHERE invocation_id = $1",
    )
    .bind(raced.invocation)
    .fetch_optional(&pool)
    .await
    .expect("load raced session status");
    assert_ne!(final_race_status.as_deref(), Some("active"));
    match race_issue {
        Ok(_) => assert_eq!(final_race_status.as_deref(), Some("revoked")),
        Err(RuntimeAuthorityError::SessionNotPending) => {
            assert_eq!(final_race_status, None);
        }
        Err(error) => panic!("unexpected issuance result in race: {error:?}"),
    }

    // Exercise both lock orderings deterministically in addition to the
    // concurrent race above: issuance may win and then be revoked, or
    // terminal completion may win and prevent session creation entirely.
    let issue_first = seed_fixture(&pool, "http.service.v1").await;
    let issue_first_request = request(
        &issue_first,
        OffsetDateTime::now_utc(),
        Duration::minutes(10),
    );
    issuer
        .issue_gateway_service(issue_first_request)
        .await
        .expect("issuance-first ordering");
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, 'completed')")
            .bind(issue_first.invocation)
            .fetch_one(&pool)
            .await
            .expect("complete issuance-first invocation")
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM gateway_runtime_authority_sessions WHERE invocation_id = $1",
        )
        .bind(issue_first.invocation)
        .fetch_one(&pool)
        .await
        .expect("load issuance-first status"),
        "revoked"
    );

    let complete_first = seed_fixture(&pool, "http.service.v1").await;
    let complete_first_request = request(
        &complete_first,
        OffsetDateTime::now_utc(),
        Duration::minutes(10),
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, 'completed')")
            .bind(complete_first.invocation)
            .fetch_one(&pool)
            .await
            .expect("complete completion-first invocation")
    );
    assert_eq!(
        issuer.issue_gateway_service(complete_first_request).await,
        Err(RuntimeAuthorityError::SessionNotPending),
        "completion-first ordering must reject issuance"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM gateway_runtime_authority_sessions
              WHERE invocation_id = $1",
        )
        .bind(complete_first.invocation)
        .fetch_one(&pool)
        .await
        .expect("count completion-first sessions"),
        0
    );

    let batched_expiry = seed_fixture(&pool, "http.service.v1").await;
    let batched_sessions = seed_expired_host_sessions(&pool, &batched_expiry, 129).await;
    assert_eq!(
        repository
            .expire(OffsetDateTime::now_utc())
            .await
            .expect("expire host sessions across bounded batches"),
        129
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM gateway_runtime_authority_sessions
              WHERE id = ANY($1) AND status = 'expired'",
        )
        .bind(&batched_sessions)
        .fetch_one(&pool)
        .await
        .expect("count batched expired sessions"),
        129
    );

    assert_eq!(
        issuer.issue_gateway(service_request).await,
        Err(RuntimeAuthorityError::IdentityMismatch),
        "guest issuance must reject a service revision"
    );

    let guest = seed_fixture(&pool, "http.v1").await;
    let guest_request = request(&guest, OffsetDateTime::now_utc(), Duration::minutes(10));
    let issued_guest = issuer
        .issue_gateway(guest_request)
        .await
        .expect("issue ordinary guest-handoff session");
    assert_eq!(issued_guest.status, RuntimeSessionStatus::PendingHandoff);
    assert_eq!(creates.load(Ordering::SeqCst), 1);
    let guest_shape: (String, Option<Vec<u8>>, String, Option<OffsetDateTime>) = sqlx::query_as(
        "SELECT admission_mode, credential_hash, status, acknowledged_at
             FROM gateway_runtime_authority_sessions WHERE id = $1",
    )
    .bind(guest.invocation)
    .fetch_one(&pool)
    .await
    .expect("load guest session shape");
    assert_eq!(guest_shape.0, "guest_handoff");
    assert_eq!(guest_shape.1.map(|value| value.len()), Some(32));
    assert_eq!(guest_shape.2, "pending_handoff");
    assert_eq!(guest_shape.3, None);
    assert_eq!(
        issuer.issue_gateway_service(guest_request).await,
        Err(RuntimeAuthorityError::IdentityMismatch),
        "host-mediated issuance must reject a stateless revision"
    );
    let acknowledged_guest = repository
        .acknowledge(
            RuntimeSessionId::from_uuid(guest.invocation),
            RuntimeCredentialGeneration::INITIAL,
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("acknowledge ordinary guest session");
    assert_eq!(acknowledged_guest.status, RuntimeSessionStatus::Active);
    assert!(acknowledged_guest.acknowledged_at.is_some());
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, 'completed')")
            .bind(guest.invocation)
            .fetch_one(&pool)
            .await
            .expect("complete ordinary guest invocation")
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1",
        )
        .bind(guest.invocation)
        .fetch_one(&pool)
        .await
        .expect("load ordinary guest session after completion"),
        "active",
        "stateless completion retains its existing session semantics"
    );

    let expiring = seed_fixture(&pool, "http.service.v1").await;
    let issued_at = OffsetDateTime::now_utc();
    let expires_at = issued_at + Duration::minutes(1);
    issuer
        .issue_gateway_service(request(&expiring, issued_at, expires_at - issued_at))
        .await
        .expect("issue expiring host-mediated session");
    let expiring_lease = seed_gateway_lease(&pool, &expiring).await;
    let expired = repository
        .expire(expires_at + Duration::seconds(1))
        .await
        .expect("expire host-mediated session");
    assert!(expired >= 1);
    let expired_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(expiring.invocation)
            .fetch_one(&pool)
            .await
            .expect("load expired host-mediated session");
    assert_eq!(expired_status, "expired");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM gateway_secret_leases WHERE id = $1",)
            .bind(expiring_lease)
            .fetch_one(&pool)
            .await
            .expect("load expired gateway lease"),
        "expired"
    );
}

fn request(
    fixture: &Fixture,
    issued_at: OffsetDateTime,
    lifetime: Duration,
) -> GatewayRuntimeSessionRequest {
    GatewayRuntimeSessionRequest {
        invocation_id: capability_domain::GatewayInvocationId::from_uuid(fixture.invocation),
        gateway_id: fixture.gateway,
        gateway_revision_id: fixture.revision,
        issued_at,
        expires_at: issued_at + lifetime,
    }
}

// Keep the complete foreign-key fixture in one helper so each assertion uses
// the same durable gateway shape.
#[allow(clippy::too_many_lines)]
async fn seed_fixture(pool: &sqlx::PgPool, handler_contract: &str) -> Fixture {
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
    let service_columns = if handler_contract == "http.service.v1" {
        "18080, '/ready', '/health'"
    } else {
        "NULL, NULL, NULL"
    };
    let secret_slots = if handler_contract == "http.service.v1" {
        "'{hook}'"
    } else {
        "'{}'"
    };
    let revision_sql = format!(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, 'public', '{{}}', {secret_slots}, {service_columns}, $6, $7)"
    );
    sqlx::query(&revision_sql)
        .bind(revision)
        .bind(gateway)
        .bind(project)
        .bind(repository)
        .bind(handler_contract)
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

async fn seed_expired_host_sessions(
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

// This helper intentionally creates the full secret/binding/rule/lease chain
// needed to exercise the database triggers rather than bypassing them.
#[allow(clippy::too_many_lines)]
async fn seed_gateway_lease(pool: &sqlx::PgPool, fixture: &Fixture) -> Uuid {
    let secret = Uuid::new_v4();
    let version = Uuid::new_v4();
    let grant = Uuid::new_v4();
    let import = Uuid::new_v4();
    let binding = Uuid::new_v4();
    let rule = Uuid::new_v4();
    let lease = Uuid::new_v4();
    let name = format!("runtime_{:032x}", secret.as_u128());
    sqlx::query(
        "INSERT INTO secrets
            (id, owner_organization_id, project_id, name, status,
             allowed_delivery_modes, created_by)
         VALUES ($1, $2, $3, $4, 'active', ARRAY['brokered'], $5)",
    )
    .bind(secret)
    .bind(fixture.organization)
    .bind(fixture.project)
    .bind(name)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("gateway secret");
    sqlx::query(
        "INSERT INTO secret_versions
            (id, secret_id, sequence, status, algorithm, key_reference,
             data_nonce, ciphertext, wrap_nonce, wrapped_data_key,
             associated_data_hash, content_length, created_by)
         VALUES ($1, $2, 1, 'active', 'AES-256-GCM+AES-256-GCM-KW/v1',
                 'test/key', decode(repeat('01', 12), 'hex'),
                 decode('01', 'hex'), decode(repeat('02', 12), 'hex'),
                 decode('03', 'hex'), decode(repeat('04', 32), 'hex'), 1, $3)",
    )
    .bind(version)
    .bind(secret)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("gateway secret version");
    sqlx::query("UPDATE secrets SET active_version_id = $2 WHERE id = $1")
        .bind(secret)
        .bind(version)
        .execute(pool)
        .await
        .expect("active gateway secret version");
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
    .expect("gateway secret grant");
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
    .bind(format!("runtime_{:032x}", import.as_u128()))
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("gateway secret import");
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
    .expect("gateway secret binding");
    sqlx::query(
        "INSERT INTO gateway_brokered_secret_rules
            (id, binding_id, gateway_revision_id, gateway_route_id,
             header_name, normalized_hash)
         SELECT $1, $2, $3, route.id, 'x-hook-secret', $4
           FROM gateway_routes AS route
          WHERE route.gateway_revision_id = $3",
    )
    .bind(rule)
    .bind(binding)
    .bind(fixture.revision)
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("gateway secret rule");
    sqlx::query(
        "INSERT INTO gateway_secret_leases
            (id, runtime_session_id, invocation_id, binding_id,
             secret_version_id, rule_id, status, expires_at)
         SELECT $1, session.id, session.invocation_id, $2, $3, $4,
                'active', session.expires_at
           FROM gateway_runtime_authority_sessions AS session
          WHERE session.id = $5",
    )
    .bind(lease)
    .bind(binding)
    .bind(version)
    .bind(rule)
    .bind(fixture.invocation)
    .execute(pool)
    .await
    .expect("gateway secret lease");
    lease
}

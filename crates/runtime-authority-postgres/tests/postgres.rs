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
    gateway: Uuid,
    revision: Uuid,
    invocation: Uuid,
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
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE version = 71")
            .fetch_one(&pool)
            .await
            .expect("confirm migration 0071 applied to the connected database");
    assert_eq!(migration_version, 71);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 migration=71");

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

    let expiring = seed_fixture(&pool, "http.service.v1").await;
    let issued_at = OffsetDateTime::now_utc();
    let expires_at = issued_at + Duration::minutes(1);
    issuer
        .issue_gateway_service(request(&expiring, issued_at, expires_at - issued_at))
        .await
        .expect("issue expiring host-mediated session");
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
    let revision_sql = format!(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, 'public', '{{}}', '{{}}', {service_columns}, $6, $7)"
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
    sqlx::query("INSERT INTO gateway_invocations (id, gateway_id, gateway_revision_id, gateway_route_id, project_id, request_id, outcome) VALUES ($1, $2, $3, $4, $5, $6, 'accepted')")
        .bind(invocation)
        .bind(gateway)
        .bind(revision)
        .bind(route)
        .bind(project)
        .bind(Uuid::new_v4())
        .execute(pool)
        .await
        .expect("gateway invocation");
    Fixture {
        gateway,
        revision,
        invocation,
    }
}

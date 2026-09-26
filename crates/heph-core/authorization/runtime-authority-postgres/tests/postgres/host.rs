use super::{
    fixtures, lease,
    support::{self, TestContext},
};
use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, RuntimeCredential, RuntimeCredentialGeneration,
    RuntimeInvocation, RuntimeSessionId, RuntimeSessionIdentity, RuntimeSessionStatus,
    WorkloadKind, WorkloadPrincipal,
};
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, NewRuntimeSession,
    RuntimeAuthorityError, RuntimeSessionRepository,
};
use std::sync::{Arc, atomic::Ordering};
use time::{Duration, OffsetDateTime};

// The host-mediated phase keeps its transaction and trigger assertions together so the real PostgreSQL lifecycle proof remains readable as one scenario.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn exercise(context: &TestContext) -> (fixtures::Fixture, GatewayRuntimeSessionRequest) {
    let pool = context.pool.clone();
    let issuer = Arc::clone(&context.issuer);
    let repository = &context.repository;
    let service = fixtures::seed_fixture(&pool, "http.service.v1").await;
    let service_request =
        support::request(&service, OffsetDateTime::now_utc(), Duration::minutes(10));
    let first = issuer
        .issue_gateway_service(service_request)
        .await
        .expect("issue host-mediated service session");
    let first_lease = lease::seed_gateway_lease(&pool, &service).await;
    assert_eq!(first.status, RuntimeSessionStatus::Active);
    assert_eq!(first.acknowledged_at, None);
    assert_eq!(context.creates.load(Ordering::SeqCst), 0);

    let concurrent = fixtures::seed_fixture(&pool, "http.service.v1").await;
    let concurrent_request = support::request(
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
    (service, service_request)
}

use super::{
    fixtures, lease,
    support::{self, TestContext},
};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId, RuntimeSessionStatus};
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, RuntimeAuthorityError, RuntimeSessionRepository,
};
use std::sync::atomic::Ordering;
use time::{Duration, OffsetDateTime};

// The guest phase keeps admission, acknowledgement, completion, and expiry assertions together to prove the cross-mode boundary in one fixture.
#[allow(clippy::cognitive_complexity)]
pub async fn exercise(context: &TestContext) {
    let pool = context.pool.clone();
    let issuer = &context.issuer;
    let repository = &context.repository;
    let creates = &context.creates;
    let guest = fixtures::seed_fixture(&pool, "http.v1").await;
    let guest_request = support::request(&guest, OffsetDateTime::now_utc(), Duration::minutes(10));
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

    let expiring = fixtures::seed_fixture(&pool, "http.service.v1").await;
    let issued_at = OffsetDateTime::now_utc();
    let expires_at = issued_at + Duration::minutes(1);
    issuer
        .issue_gateway_service(support::request(
            &expiring,
            issued_at,
            expires_at - issued_at,
        ))
        .await
        .expect("issue expiring host-mediated session");
    let expiring_lease = lease::seed_gateway_lease(&pool, &expiring).await;
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

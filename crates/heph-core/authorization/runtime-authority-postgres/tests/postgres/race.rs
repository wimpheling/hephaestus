use super::{
    fixtures, lease,
    support::{self, TestContext},
};
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, RuntimeAuthorityError,
    RuntimeSessionRepository,
};
use std::sync::Arc;
use time::{Duration, OffsetDateTime};

// The race phase keeps both lock orderings and bounded expiry assertions together so they exercise one shared PostgreSQL fixture.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn exercise(context: &TestContext, service_request: &GatewayRuntimeSessionRequest) {
    let pool = context.pool.clone();
    let issuer = Arc::clone(&context.issuer);
    let repository = &context.repository;
    for outcome in ["completed", "failed", "timed_out"] {
        let terminal = fixtures::seed_fixture(&pool, "http.service.v1").await;
        let terminal_request =
            support::request(&terminal, OffsetDateTime::now_utc(), Duration::minutes(10));
        issuer
            .issue_gateway_service(terminal_request)
            .await
            .expect("issue terminal-cleanup service session");
        lease::seed_gateway_lease(&pool, &terminal).await;
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

    let raced = fixtures::seed_fixture(&pool, "http.service.v1").await;
    let raced_request = support::request(&raced, OffsetDateTime::now_utc(), Duration::minutes(10));
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
    let issue_first = fixtures::seed_fixture(&pool, "http.service.v1").await;
    let issue_first_request = support::request(
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

    let complete_first = fixtures::seed_fixture(&pool, "http.service.v1").await;
    let complete_first_request = support::request(
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

    let batched_expiry = fixtures::seed_fixture(&pool, "http.service.v1").await;
    let batched_sessions = fixtures::seed_expired_host_sessions(&pool, &batched_expiry, 129).await;
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
        issuer.issue_gateway(*service_request).await,
        Err(RuntimeAuthorityError::IdentityMismatch),
        "guest issuance must reject a service revision"
    );
}

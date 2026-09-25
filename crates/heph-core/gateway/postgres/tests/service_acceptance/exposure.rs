use super::support_database::{test_pool, wait_for_gateway_lock};
use super::support_fixture::{
    invocation_count, invocation_outcome, seed_fixture, seed_fixture_with_exposure, session_count,
    session_status,
};
use super::support_types::{RecordingIssuer, build_authority, limits, route};
use gateway_domain::{GatewayInvocationRecorder, GatewayRouteResolver};
use gateway_postgres::PostgresGatewayEdgeAuthority;
use serial_test::serial;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

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

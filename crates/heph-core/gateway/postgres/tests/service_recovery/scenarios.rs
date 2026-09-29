//! Service recovery and fencing scenarios.

use super::support_database::{test_pool, worker_pool};
use super::support_invocation::{
    insert_host_session, insert_invocation, lease_status, outcome, session_status,
};
use super::support_lease::insert_lease;
use super::support_seed::{seed_fixture, seed_fixture_with_expired_instance};
use super::support_types::authority;
use gateway_domain::{GatewayServiceOwner, GatewayServiceOwnership};
use gateway_postgres::PostgresGatewayServiceOwnership;
use serial_test::serial;
use std::time::Duration as StdDuration;
use time::{Duration, OffsetDateTime};

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_recovery_terminalizes_only_abandoned_host_invocations() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let service = seed_fixture(&pool, "http.service.v1").await;
    let worker = worker_pool().await;
    let authority = authority(worker.clone());

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

    let processed = authority
        .recover_abandoned_service_invocations(now)
        .await
        .expect("recover abandoned service invocations");
    assert!(
        processed >= 4,
        "expected at least this test's four abandoned invocations to be recovered, got {processed}"
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
    let worker = worker_pool().await;
    let authority = authority(worker.clone());
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
    let first_batch = authority
        .recover_abandoned_service_invocations(now)
        .await
        .expect("recover first bounded batch");
    assert_eq!(first_batch, 128);
    assert_eq!(outcome(&pool, invocations[0]).await, "accepted");
    lock.rollback().await.expect("release invocation lock");
    tokio::time::timeout(StdDuration::from_secs(30), async {
        loop {
            let processed = authority
                .recover_abandoned_service_invocations(now)
                .await
                .expect("recover next bounded batch");
            assert!(processed <= 128);
            let timed_out: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM gateway_invocations
                  WHERE id = ANY($1) AND outcome = 'timed_out'",
            )
            .bind(&invocations)
            .fetch_one(&pool)
            .await
            .expect("count recovered invocations");
            if timed_out == 129 {
                break;
            }
            assert!(processed > 0, "recovery made no progress");
        }
    })
    .await
    .expect("recover all owned invocations in bounded passes");
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

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_recovery_reaps_ineligible_instance_bindings_but_preserves_draining() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let worker = worker_pool().await;

    let clock_fixture = seed_fixture(&pool, "http.service.v1").await;
    let clock_invocation = insert_invocation(&pool, clock_fixture, now).await;
    let future = now + Duration::hours(1);
    let clock_session = insert_host_session(
        &pool,
        clock_fixture,
        clock_invocation,
        now,
        future + Duration::minutes(10),
    )
    .await;
    authority(worker.clone())
        .recover_abandoned_service_invocations(future)
        .await
        .expect("recover with caller clock ahead");
    assert_eq!(outcome(&pool, clock_invocation).await, "accepted");
    assert_eq!(session_status(&pool, clock_session).await, "active");

    let invalid_state_fixture = seed_fixture(&pool, "http.service.v1").await;
    let invalid_state =
        insert_invocation(&pool, invalid_state_fixture, now - Duration::minutes(10)).await;
    let invalid_state_session = insert_host_session(
        &pool,
        invalid_state_fixture,
        invalid_state,
        now,
        now + Duration::minutes(10),
    )
    .await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'failed'
          WHERE id = $1",
    )
    .bind(invalid_state_fixture.service_instance)
    .execute(&pool)
    .await
    .expect("mark instance failed");

    let stale_fence_fixture = seed_fixture_with_expired_instance(&pool).await;
    let stale_fence =
        insert_invocation(&pool, stale_fence_fixture, now - Duration::minutes(10)).await;
    let stale_fence_session = insert_host_session(
        &pool,
        stale_fence_fixture,
        stale_fence,
        now,
        now + Duration::minutes(10),
    )
    .await;
    let ownership = PostgresGatewayServiceOwnership::new(worker.clone());
    let owner = GatewayServiceOwner::new("recovery-host", stale_fence_fixture.owner)
        .expect("valid recovery owner");
    let recovered = ownership
        .claim_expired(&owner, StdDuration::from_secs(600), 128)
        .await
        .expect("recover expired service instance");
    let recovered_stale_fence = recovered
        .iter()
        .find(|lease| Some(lease.identity.instance_id) == stale_fence_fixture.service_instance)
        .expect("recover stale-fence fixture instance");
    assert_eq!(recovered_stale_fence.fencing_token, 2);

    let expired_fixture = seed_fixture_with_expired_instance(&pool).await;
    let expired_instance =
        insert_invocation(&pool, expired_fixture, now - Duration::minutes(10)).await;
    let expired_instance_session = insert_host_session(
        &pool,
        expired_fixture,
        expired_instance,
        now,
        now + Duration::minutes(10),
    )
    .await;
    let (expired_state, expired_at): (String, OffsetDateTime) = sqlx::query_as(
        "SELECT state, lease_expires_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(expired_fixture.service_instance)
    .fetch_one(&pool)
    .await
    .expect("read expired ready instance");
    assert_eq!(expired_state, "ready");
    assert!(expired_at <= now);

    let draining_fixture = seed_fixture(&pool, "http.service.v1").await;
    let draining = insert_invocation(&pool, draining_fixture, now - Duration::minutes(10)).await;
    let draining_session = insert_host_session(
        &pool,
        draining_fixture,
        draining,
        now,
        now + Duration::minutes(10),
    )
    .await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'draining'
          WHERE id = $1",
    )
    .bind(draining_fixture.service_instance)
    .execute(&pool)
    .await
    .expect("mark instance draining");

    let processed = authority(worker)
        .recover_abandoned_service_invocations(now)
        .await
        .expect("recover ineligible instance bindings");
    assert!(
        processed >= 3,
        "expected the three ineligible bindings to be reaped"
    );
    for invocation in [invalid_state, expired_instance, stale_fence] {
        assert_eq!(outcome(&pool, invocation).await, "timed_out");
    }
    assert_eq!(outcome(&pool, draining).await, "accepted");
    for session in [
        invalid_state_session,
        expired_instance_session,
        stale_fence_session,
    ] {
        assert_eq!(session_status(&pool, session).await, "revoked");
    }
    assert_eq!(session_status(&pool, draining_session).await, "active");
}

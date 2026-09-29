use super::support_database::{test_pool, wait_for_gateway_lock};
use super::support_fixture::{
    active_lease_count, invocation_count, invocation_outcome, latest_invocation, seed_fixture,
    seed_fixture_with_expiry, seed_fixture_with_lease, session_count, session_status,
};
use super::support_types::{FailingIssuer, GhostIssuer, RecordingIssuer, build_authority, route};
use gateway_domain::GatewayInvocationRecorder;
use serial_test::serial;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;
use uuid::Uuid;

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

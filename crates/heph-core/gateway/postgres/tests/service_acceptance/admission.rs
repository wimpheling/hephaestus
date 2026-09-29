use super::support_database::{
    test_pool, wait_for_gateway_lock, wait_for_no_gateway_acceptance_sessions,
};
use super::support_fixture::{insert_service_invocation, seed_fixture};
use super::support_types::{GhostIssuer, RecordingIssuer, build_authority, route};
use gateway_domain::GatewayInvocationRecorder;
use serial_test::serial;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

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

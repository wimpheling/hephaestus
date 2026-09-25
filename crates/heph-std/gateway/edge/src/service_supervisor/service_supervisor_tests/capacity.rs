use super::*;
#[test]
fn start_reserves_capacity_before_claim_future_is_polled() {
    let mut supervisor = supervisor();
    let request = request(Uuid::new_v4(), Uuid::new_v4());
    let handle = supervisor.start(request).expect("reservation");
    assert_eq!(
        handle.subscribe().borrow().to_owned(),
        GatewayServiceSupervisorJobStatus::Claiming
    );
    assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
    assert_eq!(supervisor.capacity_snapshot().starting_instances, 1);
    drop(supervisor);
}

#[test]
fn startup_capacity_is_bounded_before_any_claim_is_polled() {
    let mut supervisor = supervisor();
    for _ in 0..2 {
        supervisor
            .start(request(Uuid::new_v4(), Uuid::new_v4()))
            .expect("startup slot");
    }
    let error = supervisor
        .start(request(Uuid::new_v4(), Uuid::new_v4()))
        .expect_err("third startup must wait");
    assert_eq!(
        error,
        GatewayServiceSupervisorError::Capacity(
            GatewayServiceCapacityError::StartupCapacityExhausted
        )
    );
    drop(supervisor);
}

#[tokio::test]
async fn cancellation_before_first_poll_releases_without_claiming() {
    let ownership = Arc::new(Noop::default());
    let mut supervisor = supervisor_with(Arc::clone(&ownership));
    let handle = supervisor
        .start(request(Uuid::new_v4(), Uuid::new_v4()))
        .expect("reservation");
    handle.cancel();
    let event = supervisor.poll().await.expect("cancelled job");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Cancelled);
    assert!(event.capacity_released);
    assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 0);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    drop(supervisor);
}

#[tokio::test]
async fn expired_queued_deadline_releases_without_claiming() {
    let ownership = Arc::new(Noop::default());
    let policy = GatewayServiceSupervisorPolicy {
        instance: crate::ServiceInstancePolicy::new(
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(1),
            std::time::Duration::from_secs(1),
        ),
        ..GatewayServiceSupervisorPolicy::default()
    };
    let mut supervisor = supervisor_with_policy(
        Arc::clone(&ownership),
        GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner"),
        policy,
    );
    supervisor
        .start(request(Uuid::new_v4(), Uuid::new_v4()))
        .expect("reservation");
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let event = supervisor.poll().await.expect("expired job");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Failed);
    assert!(event.capacity_released);
    assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 0);
    drop(supervisor);
}

// `shutdown` consumes the supervisor after joining every owned future; the
// nursery lint cannot see that this is the deliberate final drop point.

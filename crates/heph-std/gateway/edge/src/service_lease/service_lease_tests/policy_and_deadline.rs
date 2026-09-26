use super::*;

#[test]
fn rejects_invalid_policy() {
    assert!(
        GatewayServiceLeasePolicy {
            lease_duration: Duration::from_secs(1),
            renewal_interval: Duration::from_secs(1),
        }
        .validate()
        .is_err()
    );
    let owner = owner();
    let mut malformed = lease(&owner, GatewayServiceInstanceState::Starting);
    malformed.vm_id = "wrong-vm".to_owned();
    malformed.lease_expires_at = malformed.heartbeat_at;
    let result = GatewayServiceLeaseMonitor::new(
        MockOwnership::new(Vec::new()),
        malformed,
        owner,
        policy(),
        Instant::now() + Duration::from_secs(1),
    );
    assert!(matches!(
        result,
        Err(GatewayServiceLeaseError::InvalidIdentity)
    ));
}

#[tokio::test(start_paused = true)]
async fn renewal_latency_cannot_extend_prior_deadline() {
    let owner = owner();
    let initial = lease(&owner, GatewayServiceInstanceState::Starting);
    let ownership = MockOwnership::new(vec![RenewAction::Success {
        lease: initial.clone(),
        delay: Duration::from_millis(100),
    }]);
    let started = ownership.started.clone();
    let (monitor, control) = GatewayServiceLeaseMonitor::new(
        ownership.clone(),
        initial,
        owner,
        policy(),
        Instant::now() + Duration::from_millis(50),
    )
    .expect("monitor");
    let status = control.subscribe();
    let task = tokio::spawn(monitor.run());
    tokio::task::yield_now().await;
    let entered = started.notified();
    tokio::time::advance(Duration::from_millis(20)).await;
    entered.await;
    assert_eq!(ownership.calls.load(Ordering::Relaxed), 1);
    tokio::time::advance(Duration::from_millis(30)).await;
    assert_eq!(
        task.await.expect("monitor task"),
        GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Expired)
    );
    assert_eq!(
        *status.borrow(),
        GatewayServiceLeaseStatus::Lost(GatewayServiceLeaseLossReason::Expired)
    );
    drop(control);
}

#[tokio::test(start_paused = true)]
async fn unavailable_retries_only_until_expiry() {
    let owner = owner();
    let initial = lease(&owner, GatewayServiceInstanceState::Starting);
    let ownership = MockOwnership::new(vec![RenewAction::Failure {
        error: GatewayServiceOwnershipError::Unavailable,
        delay: Duration::ZERO,
    }]);
    let started = ownership.started.clone();
    let initial_deadline = Instant::now() + Duration::from_millis(50);
    let (monitor, control) = GatewayServiceLeaseMonitor::new(
        ownership.clone(),
        initial,
        owner,
        GatewayServiceLeasePolicy {
            lease_duration: Duration::from_millis(80),
            renewal_interval: Duration::from_millis(10),
        },
        initial_deadline,
    )
    .expect("monitor");
    let status = control.subscribe();
    let task = tokio::spawn(monitor.run());
    tokio::task::yield_now().await;
    let entered = started.notified();
    tokio::time::advance(Duration::from_millis(10)).await;
    entered.await;
    tokio::task::yield_now().await;
    assert_eq!(ownership.calls.load(Ordering::Relaxed), 1);
    assert!(
        matches!(&*status.borrow(), GatewayServiceLeaseStatus::Active {
        deadline,
        last_retry: Some(GatewayServiceLeaseRetryReason::Unavailable),
        ..
    } if *deadline == initial_deadline)
    );
    let retry_entered = started.notified();
    tokio::time::advance(Duration::from_millis(10)).await;
    retry_entered.await;
    assert_eq!(ownership.calls.load(Ordering::Relaxed), 2);
    tokio::time::advance(Duration::from_millis(30)).await;
    assert_eq!(
        task.await.expect("monitor task"),
        GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Expired)
    );
    assert_eq!(
        *status.borrow(),
        GatewayServiceLeaseStatus::Lost(GatewayServiceLeaseLossReason::Expired)
    );
    drop(control);
}

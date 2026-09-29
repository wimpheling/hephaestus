use super::*;

#[tokio::test(start_paused = true)]
async fn stale_and_malformed_renewals_are_terminal() {
    let owner2 = owner();
    let initial = lease(&owner2, GatewayServiceInstanceState::Starting);
    let mut malformed = initial.clone();
    malformed.fencing_token += 1;
    let (monitor, control) = GatewayServiceLeaseMonitor::new(
        MockOwnership::new(vec![RenewAction::Success {
            lease: malformed,
            delay: Duration::ZERO,
        }]),
        initial.clone(),
        owner2.clone(),
        policy(),
        Instant::now() + Duration::from_millis(80),
    )
    .expect("monitor");
    let task = tokio::spawn(monitor.run());
    tokio::time::advance(Duration::from_millis(20)).await;
    assert_eq!(
        task.await.expect("monitor task"),
        GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Invalid)
    );
    drop(control);

    let mut changed_identity = initial.clone();
    changed_identity.identity.instance_id = Uuid::new_v4();
    assert_invalid_renewal(owner2.clone(), initial.clone(), changed_identity).await;
    let mut changed_owner = initial.clone();
    changed_owner.owner_host_id = "other-host".to_owned();
    assert_invalid_renewal(owner2.clone(), initial.clone(), changed_owner).await;
    let mut changed_daemon = initial.clone();
    changed_daemon.owner_uuid = Uuid::new_v4();
    assert_invalid_renewal(owner2.clone(), initial.clone(), changed_daemon).await;
    let mut changed_vm = initial.clone();
    changed_vm.vm_id = "gateway-service-other".to_owned();
    assert_invalid_renewal(owner2.clone(), initial.clone(), changed_vm).await;

    let owner = owner();
    let initial = lease(&owner, GatewayServiceInstanceState::Starting);
    let (monitor, control) = GatewayServiceLeaseMonitor::new(
        MockOwnership::new(vec![RenewAction::Failure {
            error: GatewayServiceOwnershipError::StaleLease,
            delay: Duration::ZERO,
        }]),
        initial,
        owner,
        policy(),
        Instant::now() + Duration::from_millis(80),
    )
    .expect("monitor");
    let task = tokio::spawn(monitor.run());
    tokio::time::advance(Duration::from_millis(20)).await;
    assert_eq!(
        task.await.expect("monitor task"),
        GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Stale)
    );
    drop(control);
}

#[tokio::test(start_paused = true)]
async fn successful_renewal_preserves_lifecycle_state_changes() {
    let owner = owner();
    let initial = lease(&owner, GatewayServiceInstanceState::Starting);
    let mut ready = initial.clone();
    ready.state = GatewayServiceInstanceState::Ready;
    ready.lease_expires_at = ready.heartbeat_at + ::time::Duration::milliseconds(40);
    let mut draining = ready.clone();
    draining.state = GatewayServiceInstanceState::Draining;
    draining.lease_expires_at = draining.heartbeat_at + ::time::Duration::seconds(10);
    let ownership = MockOwnership::new(vec![
        RenewAction::Success {
            lease: ready,
            delay: Duration::from_millis(30),
        },
        RenewAction::Success {
            lease: draining,
            delay: Duration::from_millis(5),
        },
    ]);
    let started = ownership.started.clone();
    let (monitor, control) = GatewayServiceLeaseMonitor::new(
        ownership.clone(),
        initial,
        owner,
        policy(),
        Instant::now() + Duration::from_millis(200),
    )
    .expect("monitor");
    let status = control.subscribe();
    let task = tokio::spawn(monitor.run());
    tokio::task::yield_now().await;
    let entered = started.notified();
    tokio::time::advance(Duration::from_millis(20)).await;
    entered.await;
    let second_entered = started.notified();
    tokio::time::advance(Duration::from_millis(30)).await;
    tokio::task::yield_now().await;
    assert!(
        matches!(&*status.borrow(), GatewayServiceLeaseStatus::Active { lease, deadline, .. } if lease.state == GatewayServiceInstanceState::Ready
            && *deadline == ownership.call_starts.lock().expect("call start mutex")[0] + Duration::from_millis(40))
    );
    second_entered.await;
    tokio::time::advance(Duration::from_millis(5)).await;
    tokio::task::yield_now().await;
    assert!(
        matches!(&*status.borrow(), GatewayServiceLeaseStatus::Active { lease, deadline, .. } if lease.state == GatewayServiceInstanceState::Draining
            && *deadline == ownership.call_starts.lock().expect("call start mutex")[1] + policy().lease_duration)
    );
    control.stop();
    assert_eq!(
        task.await.expect("monitor task"),
        GatewayServiceLeaseRunResult::Stopped
    );
}

#[tokio::test(start_paused = true)]
async fn stopping_or_dropping_control_marks_status_unavailable() {
    let owner = owner();
    let initial = lease(&owner, GatewayServiceInstanceState::Starting);
    let ownership = MockOwnership::new(vec![RenewAction::Success {
        lease: initial.clone(),
        delay: Duration::from_secs(1),
    }]);
    let started = ownership.started.clone();
    let (monitor, control) = GatewayServiceLeaseMonitor::new(
        ownership,
        initial,
        owner,
        policy(),
        Instant::now() + Duration::from_secs(1),
    )
    .expect("monitor");
    let status = control.subscribe();
    let task = tokio::spawn(monitor.run());
    tokio::task::yield_now().await;
    let entered = started.notified();
    tokio::time::advance(Duration::from_millis(20)).await;
    entered.await;
    drop(control);
    assert_eq!(*status.borrow(), GatewayServiceLeaseStatus::Stopped);
    assert_eq!(
        task.await.expect("monitor task"),
        GatewayServiceLeaseRunResult::Stopped
    );
}

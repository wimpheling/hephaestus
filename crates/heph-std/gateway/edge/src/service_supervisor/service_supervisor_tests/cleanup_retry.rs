use super::*;
#[tokio::test]
async fn terminal_cleanup_can_retry_with_the_same_capacity_and_vm() {
    let (mut supervisor, provider, _ownership, initial_request, _accepted) = ready_supervisor(true);
    let handle = supervisor.start(initial_request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;
    handle.cancel();
    let first = supervisor.poll().await.expect("initial cleanup result");
    assert_eq!(first.status, GatewayServiceSupervisorJobStatus::Cancelled);
    assert!(!first.capacity_released);
    let retained = provider.last_vm.lock().expect("last VM").clone();
    let recorded = supervisor
        .records
        .get(&first.job_id)
        .and_then(|record| record.completion.as_ref())
        .and_then(|completion| completion.coordinator_failure.as_ref())
        .and_then(|failure| failure.vm.as_ref())
        .expect("recorded retained VM");
    assert!(Arc::ptr_eq(
        retained.as_ref().expect("retained VM"),
        recorded
    ));
    provider.fail_destroy.store(false, Ordering::Relaxed);
    supervisor
        .retry_cleanup(first.job_id)
        .expect("cleanup retry scheduling");
    assert_eq!(
        handle.subscribe().borrow().to_owned(),
        GatewayServiceSupervisorJobStatus::CleanupPending
    );
    assert_eq!(
        supervisor
            .retry_cleanup(first.job_id)
            .expect_err("duplicate retry"),
        GatewayServiceSupervisorError::RetryAlreadyInFlight
    );
    let retried = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
        .await
        .expect("bounded cleanup retry")
        .expect("retry result");
    assert_eq!(retried.status, GatewayServiceSupervisorJobStatus::Settled);
    assert!(retried.capacity_released);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    assert!(Arc::ptr_eq(
        retained.as_ref().expect("retained VM"),
        provider
            .last_vm
            .lock()
            .expect("last VM")
            .as_ref()
            .expect("VM")
    ));
    assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
    assert!(!supervisor.has_pending_jobs());
}

#[tokio::test]
async fn expired_cleanup_retains_successor_until_renewal_then_retries() {
    let (mut supervisor, provider, ownership, initial_request, _accepted) = ready_supervisor(true);
    let handle = supervisor.start(initial_request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;
    handle.cancel();
    let first = supervisor.poll().await.expect("initial cleanup result");
    assert!(!first.capacity_released);
    provider.fail_destroy.store(false, Ordering::Relaxed);

    let old = supervisor
        .records
        .get(&first.job_id)
        .and_then(|record| record.completion.as_ref())
        .and_then(|completion| completion.lease.clone())
        .expect("retained lease");
    let owner = supervisor.context.owner.clone();
    let now = OffsetDateTime::now_utc();
    let mut expired_successor = old.clone();
    expired_successor.owner_host_id = owner.host_id.clone();
    expired_successor.owner_uuid = owner.owner_uuid;
    expired_successor.fencing_token = old.fencing_token + 1;
    expired_successor.state = GatewayServiceInstanceState::Stopping;
    expired_successor.heartbeat_at = now - TimeDuration::seconds(2);
    expired_successor.lease_expires_at = now - TimeDuration::seconds(1);
    let mut live_successor = expired_successor.clone();
    live_successor.fencing_token = expired_successor.fencing_token + 1;
    live_successor.heartbeat_at = now;
    live_successor.lease_expires_at = now + TimeDuration::minutes(1);
    let recovery = Arc::new(FixedExpiredRecovery {
        ownership: Arc::clone(&ownership),
        takeover: Mutex::new(VecDeque::from([
            Ok(expired_successor.clone()),
            Ok(live_successor.clone()),
        ])),
        resolution: Mutex::new(VecDeque::new()),
        takeover_calls: AtomicUsize::new(0),
        resolution_calls: AtomicUsize::new(0),
    });
    ownership.renew_stale.store(true, Ordering::Relaxed);

    supervisor
        .retry_cleanup_with_recovery(first.job_id, recovery.clone())
        .expect("expired cleanup retry");
    let retained = supervisor.poll().await.expect("expired retry result");
    assert_eq!(retained.status, GatewayServiceSupervisorJobStatus::Failed);
    assert!(!retained.capacity_released);
    assert_eq!(
        supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.as_ref()),
        Some(&expired_successor)
    );
    assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);

    ownership.renew_stale.store(false, Ordering::Relaxed);
    supervisor
        .retry_cleanup_with_recovery(first.job_id, recovery.clone())
        .expect("eventual cleanup retry");
    let cleaned = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
        .await
        .expect("bounded eventual cleanup")
        .expect("cleanup result");
    assert_eq!(cleaned.status, GatewayServiceSupervisorJobStatus::Settled);
    assert!(cleaned.capacity_released);
    assert_eq!(recovery.takeover_calls.load(Ordering::Relaxed), 2);
    assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
}

#[tokio::test]
async fn expired_cleanup_retries_lost_takeover_and_resolution_acknowledgements() {
    let (mut supervisor, provider, ownership, initial_request, _accepted) = ready_supervisor(true);
    let handle = supervisor.start(initial_request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;
    handle.cancel();
    let first = supervisor.poll().await.expect("initial cleanup result");
    assert!(!first.capacity_released);
    provider.fail_destroy.store(false, Ordering::Relaxed);

    let old = supervisor
        .records
        .get(&first.job_id)
        .and_then(|record| record.completion.as_ref())
        .and_then(|completion| completion.lease.clone())
        .expect("retained lease");
    let owner = supervisor.context.owner.clone();
    let mut successor = old.clone();
    successor.owner_host_id = owner.host_id.clone();
    successor.owner_uuid = owner.owner_uuid;
    successor.fencing_token = old.fencing_token + 1;
    successor.state = GatewayServiceInstanceState::Stopping;
    successor.heartbeat_at = OffsetDateTime::now_utc();
    successor.lease_expires_at = successor.heartbeat_at + TimeDuration::minutes(1);
    let recovery = Arc::new(FixedExpiredRecovery {
        ownership: Arc::clone(&ownership),
        takeover: Mutex::new(VecDeque::from([
            Err(GatewayServiceOwnershipError::Unavailable),
            Err(GatewayServiceOwnershipError::StaleLease),
        ])),
        resolution: Mutex::new(VecDeque::from([
            Err(GatewayServiceOwnershipError::Unavailable),
            Ok(Some(successor.clone())),
        ])),
        takeover_calls: AtomicUsize::new(0),
        resolution_calls: AtomicUsize::new(0),
    });
    ownership.renew_stale.store(true, Ordering::Relaxed);

    supervisor
        .retry_cleanup_with_recovery(first.job_id, recovery.clone())
        .expect("first recovery retry");
    let first_retry = supervisor.poll().await.expect("first retry result");
    assert_eq!(
        first_retry.status,
        GatewayServiceSupervisorJobStatus::Failed
    );
    assert!(!first_retry.capacity_released);
    assert_eq!(recovery.takeover_calls.load(Ordering::Relaxed), 1);
    assert_eq!(recovery.resolution_calls.load(Ordering::Relaxed), 1);

    supervisor
        .retry_cleanup_with_recovery(first.job_id, recovery.clone())
        .expect("second recovery retry");
    let second_retry = supervisor.poll().await.expect("second retry result");
    assert_eq!(
        second_retry.status,
        GatewayServiceSupervisorJobStatus::Failed
    );
    assert!(!second_retry.capacity_released);
    assert_eq!(recovery.takeover_calls.load(Ordering::Relaxed), 2);
    assert_eq!(recovery.resolution_calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.as_ref()),
        Some(&successor)
    );

    ownership.renew_stale.store(false, Ordering::Relaxed);
    supervisor
        .retry_cleanup_with_recovery(first.job_id, recovery)
        .expect("final cleanup retry");
    let cleaned = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
        .await
        .expect("bounded final cleanup")
        .expect("cleanup result");
    assert_eq!(cleaned.status, GatewayServiceSupervisorJobStatus::Settled);
    assert!(cleaned.capacity_released);
    assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn expired_cleanup_rejects_unrelated_successor_without_physical_work() {
    let (mut supervisor, provider, ownership, initial_request, _accepted) = ready_supervisor(true);
    let handle = supervisor.start(initial_request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;
    handle.cancel();
    let first = supervisor.poll().await.expect("initial cleanup result");
    assert!(!first.capacity_released);
    provider.fail_destroy.store(false, Ordering::Relaxed);
    ownership.renew_stale.store(true, Ordering::Relaxed);
    let retained_vm = provider.last_vm.lock().expect("last VM").clone();
    let destroy_calls = provider.destroy_calls.load(Ordering::Relaxed);

    let old = supervisor
        .records
        .get(&first.job_id)
        .and_then(|record| record.completion.as_ref())
        .and_then(|completion| completion.lease.clone())
        .expect("retained lease");
    let mut unrelated = old.clone();
    unrelated.vm_id = String::from("gateway-service-foreign");
    let recovery = Arc::new(FixedExpiredRecovery {
        ownership: Arc::clone(&ownership),
        takeover: Mutex::new(VecDeque::from([Ok(unrelated)])),
        resolution: Mutex::new(VecDeque::new()),
        takeover_calls: AtomicUsize::new(0),
        resolution_calls: AtomicUsize::new(0),
    });
    supervisor
        .retry_cleanup_with_recovery(first.job_id, recovery)
        .expect("recovery retry");
    let rejected = supervisor.poll().await.expect("recovery result");
    assert_eq!(rejected.status, GatewayServiceSupervisorJobStatus::Failed);
    assert!(!rejected.capacity_released);
    assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
    let recorded_vm = supervisor
        .records
        .get(&first.job_id)
        .and_then(|record| record.completion.as_ref())
        .and_then(|completion| completion.coordinator_failure.as_ref())
        .and_then(|failure| failure.vm.as_ref())
        .expect("recorded retained VM");
    assert!(Arc::ptr_eq(
        retained_vm.as_ref().expect("retained VM"),
        recorded_vm
    ));
    assert_eq!(
        provider.destroy_calls.load(Ordering::Relaxed),
        destroy_calls
    );
    assert_eq!(
        supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.as_ref()),
        Some(&old)
    );
}

#[tokio::test]
async fn expired_cleanup_rejects_cleaned_successor_before_physical_confirmation() {
    let (mut supervisor, provider, ownership, initial_request, _accepted) = ready_supervisor(true);
    let handle = supervisor.start(initial_request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;
    handle.cancel();
    let first = supervisor.poll().await.expect("initial cleanup result");
    assert!(!first.capacity_released);
    provider.fail_destroy.store(false, Ordering::Relaxed);
    ownership.renew_stale.store(true, Ordering::Relaxed);
    let retained_vm = provider.last_vm.lock().expect("last VM").clone();
    let destroy_calls = provider.destroy_calls.load(Ordering::Relaxed);

    let old = supervisor
        .records
        .get(&first.job_id)
        .and_then(|record| record.completion.as_ref())
        .and_then(|completion| completion.lease.clone())
        .expect("retained lease");
    let recovery = Arc::new(FixedExpiredRecovery {
        ownership: Arc::clone(&ownership),
        takeover: Mutex::new(VecDeque::from([Ok(GatewayServiceInstanceLease {
            state: GatewayServiceInstanceState::Cleaned,
            fencing_token: old.fencing_token + 1,
            ..old.clone()
        })])),
        resolution: Mutex::new(VecDeque::new()),
        takeover_calls: AtomicUsize::new(0),
        resolution_calls: AtomicUsize::new(0),
    });
    supervisor
        .retry_cleanup_with_recovery(first.job_id, recovery)
        .expect("recovery retry");
    let rejected = supervisor.poll().await.expect("recovery result");
    assert_eq!(rejected.status, GatewayServiceSupervisorJobStatus::Failed);
    assert!(!rejected.capacity_released);
    assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
    let recorded_vm = supervisor
        .records
        .get(&first.job_id)
        .and_then(|record| record.completion.as_ref())
        .and_then(|completion| completion.coordinator_failure.as_ref())
        .and_then(|failure| failure.vm.as_ref())
        .expect("recorded retained VM");
    assert!(Arc::ptr_eq(
        retained_vm.as_ref().expect("retained VM"),
        recorded_vm
    ));
    assert_eq!(
        provider.destroy_calls.load(Ordering::Relaxed),
        destroy_calls
    );
    assert_eq!(
        supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.as_ref()),
        Some(&old)
    );
}

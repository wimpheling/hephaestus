use super::*;
#[tokio::test]
async fn coordinator_success_releases_live_capacity_after_cleanup() {
    let (mut supervisor, _provider, _ownership, request, _accepted) = ready_supervisor(false);
    let handle = supervisor.start(request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;
    assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
    assert_eq!(supervisor.capacity_snapshot().starting_instances, 0);
    handle.cancel();
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
        .await
        .expect("bounded cleanup")
        .expect("cleanup result");
    assert!(event.capacity_released);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    assert!(!supervisor.has_pending_jobs());
    let shutdown = supervisor.shutdown().await;
    assert!(shutdown.unresolved.is_empty());
}

#[tokio::test]
async fn startup_handle_forwards_drain_to_ready_coordinator() {
    let (mut supervisor, _provider, _ownership, request, accepted) = ready_supervisor(false);
    let handle = supervisor.start(request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;

    accepted.store(1, Ordering::Relaxed);
    handle.request_drain();
    let mut poll = Box::pin(supervisor.poll());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut poll)
            .await
            .is_err()
    );
    assert_eq!(
        *handle.subscribe().borrow(),
        GatewayServiceSupervisorJobStatus::Ready
    );
    accepted.store(0, Ordering::Relaxed);
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), &mut poll)
        .await
        .expect("bounded graceful drain")
        .expect("drained service event");
    drop(poll);
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Settled);
    assert!(event.capacity_released);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
}

#[tokio::test]
async fn startup_handle_retains_drain_requested_before_coordinator_creation() {
    let (mut supervisor, _provider, _ownership, request, _accepted) = ready_supervisor(false);
    let handle = supervisor.start(request).expect("reservation");
    handle.request_drain();

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
        .await
        .expect("bounded pre-start drain")
        .expect("drained service event");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Settled);
    assert!(event.capacity_released);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
}

// `shutdown` consumes the supervisor after joining every owned future; the
// nursery lint cannot see that this is the deliberate final drop point.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test]
async fn cancellation_after_claim_started_retains_late_lease_and_capacity() {
    let ownership = Arc::new(Noop::default());
    ownership.block_claim.store(true, Ordering::Relaxed);
    let request = request(Uuid::new_v4(), Uuid::new_v4());
    let owner = GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner");
    let claimed = lease(request, &owner);
    *ownership.claim_result.lock().expect("claim result") = Some(Ok(claimed.clone()));
    let mut supervisor = supervisor_with_policy(
        Arc::clone(&ownership),
        owner,
        GatewayServiceSupervisorPolicy::default(),
    );
    let handle = supervisor.start(request).expect("reservation");
    let event = {
        let poll = supervisor.poll();
        tokio::pin!(poll);
        tokio::select! {
            () = ownership.claim_started.notified() => {}
            _ = &mut poll => panic!("claim should be blocked before cancellation"),
        }
        handle.cancel();
        ownership.claim_release.notify_one();
        poll.await.expect("cancelled job")
    };
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Cancelled);
    assert!(!event.capacity_released);
    let shutdown = supervisor.shutdown().await;
    assert_eq!(shutdown.unresolved.len(), 1);
    assert_eq!(shutdown.unresolved[0].request, request);
    assert_eq!(shutdown.unresolved[0].lease.as_ref(), Some(&claimed));
    assert_eq!(
        shutdown.unresolved[0].original_reason,
        Some(GatewayServiceCoordinatorFailureReason::Cancelled)
    );
}

// `shutdown` consumes the supervisor after joining every owned future; the
// nursery lint cannot see that this is the deliberate final drop point.

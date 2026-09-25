use super::*;
// The supervisor retains the late claim until the cancellation path has
// completed and released its ownership-bearing state.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test]
async fn successful_wrong_revision_claim_is_retained_without_provisioning() {
    let ownership = Arc::new(Noop::default());
    let owner = GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner");
    let requested = request(Uuid::new_v4(), Uuid::new_v4());
    let wrong = request(requested.gateway_id, Uuid::new_v4());
    let claimed = lease(wrong, &owner);
    *ownership.claim_result.lock().expect("claim result") = Some(Ok(claimed.clone()));
    let mut supervisor = supervisor_with_policy(
        Arc::clone(&ownership),
        owner,
        GatewayServiceSupervisorPolicy::default(),
    );
    supervisor.start(requested).expect("reservation");
    let event = supervisor.poll().await.expect("invalid claim job");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Failed);
    assert!(!event.capacity_released);
    let shutdown = supervisor.shutdown().await;
    assert_eq!(shutdown.unresolved.len(), 1);
    assert_eq!(shutdown.unresolved[0].lease.as_ref(), Some(&claimed));
    assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn coordinator_readiness_releases_startup_but_cleanup_failure_retains_vm() {
    let (mut supervisor, provider, _ownership, initial_request, _accepted) = ready_supervisor(true);
    let handle = supervisor.start(initial_request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;
    let snapshot = supervisor.capacity_snapshot();
    assert_eq!(snapshot.live_instances, 1);
    assert_eq!(snapshot.starting_instances, 0);
    handle.cancel();
    let event = supervisor.poll().await.expect("cleanup result");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Cancelled);
    assert!(!event.capacity_released);
    let expected_vm = provider.last_vm.lock().expect("last VM").clone();
    let shutdown = supervisor.shutdown().await;
    assert_eq!(shutdown.unresolved.len(), 1);
    let failure = shutdown.unresolved[0]
        .coordinator_failure
        .as_ref()
        .expect("retained coordinator failure");
    let actual_vm = failure.vm.as_ref().expect("retained VM");
    assert!(Arc::ptr_eq(
        actual_vm,
        expected_vm.as_ref().expect("expected VM")
    ));
    assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
    assert!(!failure.physical_cleanup_complete);
}

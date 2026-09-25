use super::*;

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keeps retained ownership and failed reporting together.
async fn cleanup_failure_retains_vm_and_materialization() {
    let identity = identity();
    let owner = GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
    let (events, _) = broadcast::channel(2);
    let vm = Arc::new(MockVm {
        id: VmId(format!("gateway-service-{}", identity.instance_id)),
        events,
        connections: Mutex::new(VecDeque::new()),
        starts: AtomicUsize::new(0),
        destroys: AtomicUsize::new(0),
        destroy_started: Notify::new(),
        destroy_release: Notify::new(),
        destroy_blocked: AtomicBool::new(false),
        destroy_fails: AtomicBool::new(true),
        wait_exit: Notify::new(),
        should_exit: AtomicBool::new(false),
        opens: AtomicUsize::new(0),
        health_activity: None,
    });
    let original_vm: Arc<dyn VmInstance> = vm.clone();
    let (client, peer) = tokio::io::duplex(4096);
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(client));
    tokio::spawn(respond(peer));
    let resolver = Arc::new(MockResolver {
        launch: launch(identity),
        started: Notify::new(),
        release: Notify::new(),
        cleanups: AtomicUsize::new(0),
    });
    let provider = Arc::new(MockProvider {
        provisions: AtomicUsize::new(0),
        vm: Some(vm.clone()),
        started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(false),
    });
    let ownership = Arc::new(MockOwnership {
        events: Mutex::new(Vec::new()),
        renewals: AtomicUsize::new(0),
        renewal_activity: None,
        renew_fails: AtomicBool::new(false),
        stopping_fails: AtomicBool::new(false),
        drain_conflict: AtomicBool::new(false),
        drain_blocked: AtomicBool::new(false),
        drain_started: Notify::new(),
        drain_release: Notify::new(),
        promote_fails: AtomicBool::new(false),
        promote_started: Notify::new(),
        promote_release: Notify::new(),
        promote_blocked: AtomicBool::new(false),
    });
    ownership.stopping_fails.store(true, Ordering::Relaxed);
    let failures = failure_store();
    *failures.error.lock().expect("failure error mutex") =
        Some(GatewayServiceFailureStoreError::Unavailable);
    let now = Instant::now();
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease(&owner, identity),
        owner,
        now + Duration::from_secs(30),
        now + Duration::from_secs(2),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership,
        failures.clone(),
        resolver.clone(),
        provider,
        Arc::new(MockTargets {
            target: None,
            count: default_count_state(),
        }),
        GatewayServiceRegistry::new(1, 1).expect("registry"),
        "service.test",
        supervisor_policy(
            ServiceInstancePolicy::new(
                Duration::from_secs(120),
                Duration::from_millis(1),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ),
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_secs(5),
            },
        ),
    )
    .expect("coordinator");
    let task = tokio::spawn(coordinator.run());
    resolver.started.notified().await;
    resolver.release.notify_one();
    let mut status = control.subscribe();
    while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
        status.changed().await.expect("coordinator status");
    }
    control.cancel();
    let failure = task.await.expect("coordinator join").expect_err("cleanup");
    assert_eq!(
        failure.reason,
        GatewayServiceCoordinatorFailureReason::Cancelled
    );
    assert!(
        failure
            .vm
            .as_ref()
            .is_some_and(|retained| Arc::ptr_eq(retained, &original_vm))
    );
    assert!(failure.materialization_owned);
    assert!(!failure.physical_cleanup_complete);
    assert!(!failure.durable_cleanup_complete);
    assert_eq!(
        failure.pending_failure.map(|failure| failure.code),
        Some(GatewayServiceFailureCode::Cleanup)
    );
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn blocked_cleanup_keeps_lease_monitor_running() {
    let identity = identity();
    let owner = GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
    let (events, _) = broadcast::channel(2);
    let vm = Arc::new(MockVm {
        id: VmId(format!("gateway-service-{}", identity.instance_id)),
        events,
        connections: Mutex::new(VecDeque::new()),
        starts: AtomicUsize::new(0),
        destroys: AtomicUsize::new(0),
        destroy_started: Notify::new(),
        destroy_release: Notify::new(),
        destroy_blocked: AtomicBool::new(true),
        destroy_fails: AtomicBool::new(false),
        wait_exit: Notify::new(),
        should_exit: AtomicBool::new(false),
        opens: AtomicUsize::new(0),
        health_activity: None,
    });
    let (client, peer) = tokio::io::duplex(4096);
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(client));
    tokio::spawn(respond(peer));
    let resolver = Arc::new(MockResolver {
        launch: launch(identity),
        started: Notify::new(),
        release: Notify::new(),
        cleanups: AtomicUsize::new(0),
    });
    let provider = Arc::new(MockProvider {
        provisions: AtomicUsize::new(0),
        vm: Some(vm.clone()),
        started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(false),
    });
    let ownership = Arc::new(MockOwnership {
        events: Mutex::new(Vec::new()),
        renewals: AtomicUsize::new(0),
        renewal_activity: None,
        renew_fails: AtomicBool::new(false),
        stopping_fails: AtomicBool::new(false),
        drain_conflict: AtomicBool::new(false),
        drain_blocked: AtomicBool::new(false),
        drain_started: Notify::new(),
        drain_release: Notify::new(),
        promote_fails: AtomicBool::new(false),
        promote_started: Notify::new(),
        promote_release: Notify::new(),
        promote_blocked: AtomicBool::new(false),
    });
    let now = Instant::now();
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease(&owner, identity),
        owner,
        now + Duration::from_secs(30),
        now + Duration::from_secs(10),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership.clone(),
        failure_store(),
        resolver.clone(),
        provider,
        Arc::new(MockTargets {
            target: None,
            count: default_count_state(),
        }),
        GatewayServiceRegistry::new(1, 1).expect("registry"),
        "service.test",
        supervisor_policy(
            ServiceInstancePolicy::new(
                Duration::from_secs(120),
                Duration::from_millis(1),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ),
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_millis(10),
            },
        ),
    )
    .expect("coordinator");
    let mut status = control.subscribe();
    let task = tokio::spawn(coordinator.run());
    resolver.started.notified().await;
    resolver.release.notify_one();
    while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
        status.changed().await.expect("coordinator status");
    }
    control.cancel();
    vm.destroy_started.notified().await;
    let renewals_at_destroy_start = ownership.renewals.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(ownership.renewals.load(Ordering::Relaxed) > renewals_at_destroy_start);
    vm.destroy_release.notify_one();
    assert!(task.await.expect("coordinator join").is_ok());
}

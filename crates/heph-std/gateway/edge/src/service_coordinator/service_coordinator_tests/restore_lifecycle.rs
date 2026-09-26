use super::*;

#[tokio::test]
async fn restore_active_preserves_a_newer_desired_revision() {
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
        now + Duration::from_secs(2),
        GatewayServiceStartupIntent::RestoreActive,
        ownership.clone(),
        failure_store(),
        resolver.clone(),
        provider,
        Arc::new(MockTargets {
            target: Some(restore_target(identity, Uuid::from_u128(5))),
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
    let mut status = control.subscribe();
    let task = tokio::spawn(coordinator.run());
    resolver.started.notified().await;
    resolver.release.notify_one();
    while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
        status.changed().await.expect("coordinator status");
    }
    control.cancel();
    assert!(task.await.expect("coordinator join").is_ok());
    assert_eq!(
        ownership.events.lock().expect("events").as_slice(),
        ["starting", "ready", "stopping", "cleaned"]
    );
}

#[tokio::test(start_paused = true)]
#[allow(clippy::too_many_lines)] // Keeps blocked restore cancellation and cleanup together.
async fn lease_loss_during_restore_query_stops_promptly() {
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
        vm: Some(vm),
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
    let targets = Arc::new(BlockingTargets {
        target: Some(restore_target(identity, Uuid::from_u128(5))),
        started: Notify::new(),
        release: Notify::new(),
    });
    let now = Instant::now();
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease(&owner, identity),
        owner,
        now + Duration::from_secs(30),
        now + Duration::from_secs(30),
        GatewayServiceStartupIntent::RestoreActive,
        ownership.clone(),
        failure_store(),
        resolver.clone(),
        provider,
        targets.clone(),
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
    targets.started.notified().await;
    ownership.renew_fails.store(true, Ordering::Relaxed);
    tokio::time::advance(Duration::from_secs(6)).await;
    tokio::task::yield_now().await;
    let join = tokio::time::timeout(Duration::from_secs(1), task);
    tokio::pin!(join);
    tokio::time::advance(Duration::from_secs(1)).await;
    let failure = join
        .await
        .expect("coordinator must stop while restore query remains blocked")
        .expect("coordinator join")
        .expect_err("lease loss");
    control.cancel();
    assert_eq!(
        failure.reason,
        GatewayServiceCoordinatorFailureReason::LeaseLost
    );
    assert!(failure.vm.is_none());
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
}

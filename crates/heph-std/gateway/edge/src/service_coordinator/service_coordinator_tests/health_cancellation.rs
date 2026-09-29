use super::*;

#[tokio::test(start_paused = true)]
#[allow(clippy::too_many_lines)] // Keeps cancellation and cleanup ordering explicit.
async fn cancellation_during_blocked_health_probe_cleans_promptly() {
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
    let (ready_client, ready_peer) = tokio::io::duplex(4096);
    let (health_client, _health_peer) = tokio::io::duplex(4096);
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(ready_client));
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(health_client));
    tokio::spawn(respond(ready_peer));
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
        now + Duration::from_secs(30),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership,
        failure_store(),
        resolver.clone(),
        provider,
        Arc::new(MockTargets {
            target: None,
            count: default_count_state(),
        }),
        GatewayServiceRegistry::new(1, 1).expect("registry"),
        "service.test",
        supervisor_policy_with_health(
            ServiceInstancePolicy::new(
                Duration::from_secs(2),
                Duration::from_millis(1),
                Duration::from_millis(100),
                Duration::from_secs(1),
            ),
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_secs(5),
            },
            Duration::from_millis(1),
            3,
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
    tokio::time::advance(Duration::from_millis(1)).await;
    timeout(Duration::from_secs(1), async {
        while vm.opens.load(Ordering::Relaxed) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("blocked health probe");
    control.cancel();
    let mut task = Box::pin(task);
    let joined = tokio::time::timeout(Duration::from_secs(1), &mut task);
    tokio::pin!(joined);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(
        joined
            .await
            .expect("cancellation deadline")
            .expect("join")
            .is_ok()
    );
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
}

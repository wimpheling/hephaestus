use super::*;

pub(super) async fn respond_status_after_gate(
    mut peer: DuplexStream,
    status: u16,
    started: Arc<Notify>,
    release: Arc<Notify>,
    replied: Arc<Notify>,
) {
    let mut request = [0_u8; 512];
    let _ = peer.read(&mut request).await;
    started.notify_one();
    release.notified().await;
    let response =
        format!("HTTP/1.1 {status} Test\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
    peer.write_all(response.as_bytes())
        .await
        .expect("probe response");
    replied.notify_one();
}

#[tokio::test(start_paused = true)]
async fn startup_deadline_cancels_preparation_before_provisioning() {
    let identity = identity();
    let owner = GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
    let resolver = Arc::new(MockResolver {
        launch: launch(identity),
        started: Notify::new(),
        release: Notify::new(),
        cleanups: AtomicUsize::new(0),
    });
    let provider = Arc::new(MockProvider {
        provisions: AtomicUsize::new(0),
        vm: None,
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
    let failures = failure_store();
    let now = Instant::now();
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease(&owner, identity),
        owner,
        now + Duration::from_secs(30),
        now + Duration::from_millis(10),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership,
        failures.clone(),
        resolver.clone(),
        provider.clone(),
        Arc::new(MockTargets {
            target: None,
            count: default_count_state(),
        }),
        GatewayServiceRegistry::new(1, 1).expect("registry"),
        "service.test",
        supervisor_policy(
            ServiceInstancePolicy::new(
                Duration::from_secs(120),
                Duration::from_secs(1),
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
    let run = tokio::spawn(coordinator.run());
    resolver.started.notified().await;
    tokio::time::advance(Duration::from_millis(20)).await;
    tokio::task::yield_now().await;
    resolver.release.notify_one();
    let failure = run.await.expect("coordinator join").expect_err("deadline");
    assert_eq!(
        failure.reason,
        GatewayServiceCoordinatorFailureReason::StartupDeadline
    );
    assert_eq!(
        failures
            .reports
            .lock()
            .expect("failure reports")
            .last()
            .map(|(_, failure)| failure.code),
        Some(GatewayServiceFailureCode::Startup)
    );
    assert_eq!(provider.provisions.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    control.cancel();
}

#[tokio::test]
// The test keeps the full startup, dispatch, and cleanup ordering in one
// scenario; the added drain controls push it just over Clippy's line bound.
#[allow(clippy::too_many_lines)]
async fn readiness_registers_promotes_and_explicit_shutdown_cleans() {
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
    let failures = failure_store();
    let now = Instant::now();
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease(&owner, identity),
        owner,
        now + Duration::from_secs(30),
        now + Duration::from_secs(2),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership.clone(),
        failures.clone(),
        resolver.clone(),
        provider.clone(),
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
    let mut status = control.subscribe();
    let task = tokio::spawn(coordinator.run());
    resolver.started.notified().await;
    resolver.release.notify_one();
    while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
        status.changed().await.expect("coordinator status");
    }
    control.cancel();
    assert!(task.await.expect("coordinator join").is_ok());
    assert!(failures.reports.lock().expect("failure reports").is_empty());
    assert_eq!(provider.provisions.load(Ordering::Relaxed), 1);
    assert_eq!(vm.starts.load(Ordering::Relaxed), 1);
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    assert_eq!(
        ownership.events.lock().expect("events").as_slice(),
        ["starting", "ready", "promote", "stopping", "cleaned"]
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keeps durable reporting beside lifecycle setup.
async fn failed_readiness_never_registers_or_promotes() {
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
    tokio::spawn(respond_status(peer, 503));
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
    let failures = failure_store();
    *failures.error.lock().expect("failure error mutex") =
        Some(GatewayServiceFailureStoreError::Unavailable);
    let now = Instant::now();
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease(&owner, identity),
        owner,
        now + Duration::from_secs(30),
        now + Duration::from_secs(10),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership.clone(),
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
                Duration::from_secs(2),
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
    let failure = task
        .await
        .expect("coordinator join")
        .expect_err("readiness");
    control.cancel();
    assert_eq!(
        failure.reason,
        GatewayServiceCoordinatorFailureReason::Runtime
    );
    assert_eq!(
        failure.pending_failure.map(|failure| failure.code),
        Some(GatewayServiceFailureCode::Readiness)
    );
    assert!(failure.physical_cleanup_complete);
    assert!(!failure.durable_cleanup_complete);
    assert_eq!(vm.starts.load(Ordering::Relaxed), 1);
    assert_eq!(
        ownership.events.lock().expect("events").as_slice(),
        ["starting", "stopping"]
    );
}

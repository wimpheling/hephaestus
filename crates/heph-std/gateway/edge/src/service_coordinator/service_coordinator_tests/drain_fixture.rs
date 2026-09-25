use super::*;

pub(super) struct DrainFixture {
    pub(super) control: GatewayServiceCoordinatorControl,
    pub(super) task: tokio::task::JoinHandle<Result<(), GatewayServiceCoordinatorFailure>>,
    pub(super) status: watch::Receiver<GatewayServiceCoordinatorStatus>,
    pub(super) vm: Arc<MockVm>,
    pub(super) ownership: Arc<MockOwnership>,
    pub(super) targets: Arc<MockTargets>,
    pub(super) registry: GatewayServiceRegistry,
    pub(super) key: GatewayServiceInstanceKey,
}

// The fixture deliberately assembles the complete parent-owned lifecycle.
pub(super) async fn start_drain_fixture(
    count: Arc<MockCountState>,
    drain_conflict: bool,
    drain_blocked: bool,
    drain_timeout: Duration,
) -> DrainFixture {
    start_drain_fixture_with_logs(
        count,
        drain_conflict,
        drain_blocked,
        drain_timeout,
        false,
        None,
        Duration::from_secs(10),
    )
    .await
}

#[allow(clippy::too_many_lines)]
pub(super) async fn start_drain_fixture_with_logs(
    count: Arc<MockCountState>,
    drain_conflict: bool,
    drain_blocked: bool,
    drain_timeout: Duration,
    log_capture: bool,
    log_store: Option<Arc<dyn GatewayServiceLogStore>>,
    health_interval: Duration,
) -> DrainFixture {
    let identity = identity();
    let owner = GatewayServiceOwner::new("coordinator-drain-host", Uuid::new_v4()).expect("owner");
    let health_activity = Arc::new(Notify::new());
    let renewal_activity = Arc::new(Notify::new());
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
        health_activity: Some(Arc::clone(&health_activity)),
    });
    let (client, peer) = tokio::io::duplex(4096);
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(client));
    tokio::spawn(respond(peer));
    let mut service_launch = launch(identity);
    if log_capture {
        service_launch.service = service_launch
            .service
            .with_log_capture_mode(gateway_domain::ServiceLogCaptureMode::Application);
    }
    let resolver = Arc::new(MockResolver {
        launch: service_launch,
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
        renewal_activity: Some(Arc::clone(&renewal_activity)),
        renew_fails: AtomicBool::new(false),
        stopping_fails: AtomicBool::new(false),
        drain_conflict: AtomicBool::new(drain_conflict),
        drain_blocked: AtomicBool::new(drain_blocked),
        drain_started: Notify::new(),
        drain_release: Notify::new(),
        promote_fails: AtomicBool::new(false),
        promote_started: Notify::new(),
        promote_release: Notify::new(),
        promote_blocked: AtomicBool::new(false),
    });
    let targets = Arc::new(MockTargets {
        target: None,
        count,
    });
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
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
        targets.clone(),
        registry.clone(),
        "service.test",
        GatewayServiceSupervisorPolicy {
            drain_timeout,
            health_interval,
            lease: GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_millis(10),
            },
            instance: ServiceInstancePolicy::new(
                Duration::from_secs(2),
                Duration::from_millis(1),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ),
            ..GatewayServiceSupervisorPolicy::default()
        },
    )
    .expect("coordinator");
    let coordinator = if let Some(store) = log_store {
        coordinator.with_log_writer(GatewayServiceLogWriterConfig::new(
            store,
            ServiceLogWriterPolicy::default(),
        ))
    } else {
        coordinator
    };
    let mut status = control.subscribe();
    let task = tokio::spawn(coordinator.run());
    resolver.started.notified().await;
    resolver.release.notify_one();
    timeout(Duration::from_secs(2), async {
        while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
            status.changed().await.expect("coordinator status");
        }
    })
    .await
    .expect("ready");
    let key = GatewayServiceInstanceKey {
        identity,
        fencing_token: 1,
    };
    DrainFixture {
        control,
        task,
        status,
        vm,
        ownership,
        targets,
        registry,
        key,
    }
}

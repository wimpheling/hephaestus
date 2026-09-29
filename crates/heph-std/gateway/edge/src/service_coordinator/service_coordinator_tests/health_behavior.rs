use super::*;

#[tokio::test]
#[allow(clippy::too_many_lines)] // Sequence controlled health replies and lifecycle assertions.
async fn serving_health_survives_one_failure_and_resets_after_success() {
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
    let (failed_health_client, failed_health_peer) = tokio::io::duplex(4096);
    let (successful_health_client, successful_health_peer) = tokio::io::duplex(4096);
    let (second_failed_health_client, second_failed_health_peer) = tokio::io::duplex(4096);
    let (fourth_failed_health_client, fourth_failed_health_peer) = tokio::io::duplex(4096);
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(client));
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(failed_health_client));
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(successful_health_client));
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(second_failed_health_client));
    vm.connections
        .lock()
        .expect("connection mutex")
        .push_back(Box::new(fourth_failed_health_client));
    tokio::spawn(respond(peer));
    let first_reply = Arc::new(Notify::new());
    let second_reply = Arc::new(Notify::new());
    let third_reply = Arc::new(Notify::new());
    let fourth_started = Arc::new(Notify::new());
    let fourth_release = Arc::new(Notify::new());
    let fourth_reply = Arc::new(Notify::new());
    tokio::spawn(respond_status_notifying(
        failed_health_peer,
        503,
        first_reply.clone(),
    ));
    tokio::spawn(respond_status_notifying(
        successful_health_peer,
        200,
        second_reply.clone(),
    ));
    tokio::spawn(respond_status_notifying(
        second_failed_health_peer,
        503,
        third_reply.clone(),
    ));
    tokio::spawn(respond_status_after_gate(
        fourth_failed_health_peer,
        503,
        fourth_started.clone(),
        fourth_release.clone(),
        fourth_reply.clone(),
    ));
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
    failures.blocked.store(true, Ordering::Relaxed);
    let now = Instant::now();
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease(&owner, identity),
        owner,
        now + Duration::from_secs(30),
        now + Duration::from_secs(5),
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
        supervisor_policy_with_health(
            ServiceInstancePolicy::new(
                Duration::from_secs(2),
                Duration::from_millis(1),
                Duration::from_millis(100),
                Duration::from_secs(1),
            ),
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_millis(10),
            },
            Duration::from_millis(10),
            2,
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

    timeout(Duration::from_secs(1), async {
        while vm.opens.load(Ordering::Relaxed) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first health probe");
    first_reply.notified().await;
    assert!(!task.is_finished(), "one health failure is tolerated");
    timeout(Duration::from_secs(1), async {
        while vm.opens.load(Ordering::Relaxed) < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("successful health probe");
    second_reply.notified().await;
    timeout(Duration::from_secs(1), async {
        while vm.opens.load(Ordering::Relaxed) < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("post-reset health probe");
    third_reply.notified().await;
    timeout(Duration::from_secs(1), fourth_started.notified())
        .await
        .expect("post-reset failure must be consumed before threshold");
    assert!(
        !task.is_finished(),
        "reset keeps the first post-success failure alive while next probe is blocked"
    );
    fourth_release.notify_one();
    fourth_reply.notified().await;
    timeout(Duration::from_secs(1), failures.started.notified())
        .await
        .expect("failure report starts");
    let renewals_while_reporting = ownership.renewals.load(Ordering::Relaxed);
    timeout(Duration::from_secs(1), async {
        while ownership.renewals.load(Ordering::Relaxed) <= renewals_while_reporting {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("lease monitor must continue while failure recording is blocked");
    failures.release.notify_one();

    let failure = timeout(Duration::from_secs(1), task)
        .await
        .expect("health threshold");
    let failure = failure
        .expect("coordinator join")
        .expect_err("health threshold failure");
    control.cancel();
    assert_eq!(
        failure.reason,
        GatewayServiceCoordinatorFailureReason::Health
    );
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    assert_eq!(
        failures
            .reports
            .lock()
            .expect("failure reports")
            .last()
            .map(|(_, failure)| failure.code),
        Some(GatewayServiceFailureCode::Health)
    );
}

use super::*;

#[tokio::test]
// Keep the fixture-owned VM and coordinator control alive until the
// joined task has completed its durable cleanup transition.
#[allow(clippy::significant_drop_tightening)]
async fn lifecycle_log_writer_flushes_after_vm_teardown_before_mark_cleaned() {
    let store = Arc::new(LifecycleLogStore {
        append_started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
        records: AtomicUsize::new(0),
        responses: Mutex::new(VecDeque::new()),
        leases: Mutex::new(Vec::new()),
        events: Mutex::new(Vec::new()),
    });
    let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
    let fixture = start_drain_fixture_with_logs(
        default_count_state(),
        false,
        false,
        Duration::from_secs(2),
        true,
        Some(store_port),
        Duration::from_secs(10),
    )
    .await;
    let _ = fixture.vm.events.send(VmEvent::Log {
        stream: vm_trait::LogStream::Stdout,
        bytes: b"lifecycle log".to_vec(),
    });
    fixture.control.request_drain();
    timeout(Duration::from_secs(1), store.append_started.notified())
        .await
        .expect("durable append started");
    assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(
        fixture
            .ownership
            .events
            .lock()
            .expect("ownership events")
            .last()
            .copied(),
        Some("stopping")
    );
    store.blocked.store(false, Ordering::Relaxed);
    store.release.notify_waiters();
    assert!(fixture.task.await.expect("coordinator join").is_ok());
    assert!(store.calls.load(Ordering::Relaxed) >= 1);
    assert!(store.records.load(Ordering::Relaxed) >= 1);
    assert_eq!(
        fixture
            .ownership
            .events
            .lock()
            .expect("ownership events")
            .last()
            .copied(),
        Some("cleaned")
    );
}

#[tokio::test]
// Keep the fixture-owned VM and coordinator control alive until the
// joined task has completed its durable cleanup transition.
#[allow(clippy::significant_drop_tightening)]
async fn application_writer_flushes_while_ready_health_and_lease_renewal_continue() {
    let store = Arc::new(LifecycleLogStore {
        append_started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        records: AtomicUsize::new(0),
        responses: Mutex::new(VecDeque::new()),
        leases: Mutex::new(Vec::new()),
        events: Mutex::new(Vec::new()),
    });
    let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
    let fixture = start_drain_fixture_with_logs(
        default_count_state(),
        false,
        false,
        Duration::from_secs(2),
        true,
        Some(store_port),
        Duration::from_millis(10),
    )
    .await;
    for _ in 0..32 {
        let (client, peer) = tokio::io::duplex(4096);
        fixture
            .vm
            .connections
            .lock()
            .expect("health connections")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
    }
    let _ = fixture.vm.events.send(VmEvent::Log {
        stream: vm_trait::LogStream::Stdout,
        bytes: b"ready log".to_vec(),
    });
    timeout(Duration::from_secs(1), store.append_started.notified())
        .await
        .expect("ready append");
    assert_eq!(
        *fixture.status.borrow(),
        GatewayServiceCoordinatorStatus::Ready
    );
    assert!(store.records.load(Ordering::Relaxed) >= 1);

    store.blocked.store(true, Ordering::Relaxed);
    let _ = fixture.vm.events.send(VmEvent::Log {
        stream: vm_trait::LogStream::Stderr,
        bytes: b"blocked log".to_vec(),
    });
    timeout(Duration::from_secs(2), store.append_started.notified())
        .await
        .expect("blocked append");
    let opens_at_blocked_append = fixture.vm.opens.load(Ordering::Relaxed);
    let renewals_at_blocked_append = fixture.ownership.renewals.load(Ordering::Relaxed);
    let vm = Arc::clone(&fixture.vm);
    let ownership = Arc::clone(&fixture.ownership);
    let health_activity = Arc::clone(
        fixture
            .vm
            .health_activity
            .as_ref()
            .expect("health activity signal"),
    );
    let renewal_activity = Arc::clone(
        fixture
            .ownership
            .renewal_activity
            .as_ref()
            .expect("renewal activity signal"),
    );
    timeout(Duration::from_secs(5), async move {
        tokio::join!(
            async {
                while vm.opens.load(Ordering::Relaxed) <= opens_at_blocked_append {
                    health_activity.notified().await;
                }
            },
            async {
                while ownership.renewals.load(Ordering::Relaxed) <= renewals_at_blocked_append {
                    renewal_activity.notified().await;
                }
            },
        );
    })
    .await
    .expect("health and lease activity while append is blocked");
    assert_eq!(
        *fixture.status.borrow(),
        GatewayServiceCoordinatorStatus::Ready
    );

    store.blocked.store(false, Ordering::Relaxed);
    store.release.notify_waiters();
    fixture.control.request_drain();
    assert!(fixture.task.await.expect("coordinator join").is_ok());
    assert!(store.calls.load(Ordering::Relaxed) >= 2);
    assert!(store.records.load(Ordering::Relaxed) >= 2);
}

#[tokio::test]
// Keep the fixture-owned VM and coordinator control alive until the
// joined task has completed its durable cleanup transition.
#[allow(clippy::significant_drop_tightening)]
async fn disabled_capture_never_calls_configured_log_store() {
    let store = Arc::new(LifecycleLogStore {
        append_started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        records: AtomicUsize::new(0),
        responses: Mutex::new(VecDeque::new()),
        leases: Mutex::new(Vec::new()),
        events: Mutex::new(Vec::new()),
    });
    let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
    let fixture = start_drain_fixture_with_logs(
        default_count_state(),
        false,
        false,
        Duration::from_secs(2),
        false,
        Some(store_port),
        Duration::from_secs(10),
    )
    .await;
    let _ = fixture.vm.events.send(VmEvent::Log {
        stream: vm_trait::LogStream::Stdout,
        bytes: b"disabled log".to_vec(),
    });
    fixture.control.request_drain();
    assert!(fixture.task.await.expect("coordinator join").is_ok());
    assert_eq!(store.calls.load(Ordering::Relaxed), 0);
    assert_eq!(store.records.load(Ordering::Relaxed), 0);
}

#[tokio::test]
// Keep the fixture-owned VM and coordinator control alive until the
// joined task has completed its durable cleanup transition.
#[allow(clippy::significant_drop_tightening)]
async fn blocked_final_flush_delays_cleaned_but_not_vm_destroy() {
    let store = Arc::new(LifecycleLogStore {
        append_started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
        records: AtomicUsize::new(0),
        responses: Mutex::new(VecDeque::new()),
        leases: Mutex::new(Vec::new()),
        events: Mutex::new(Vec::new()),
    });
    let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
    let fixture = start_drain_fixture_with_logs(
        default_count_state(),
        false,
        false,
        Duration::from_secs(2),
        true,
        Some(store_port),
        Duration::from_secs(10),
    )
    .await;
    let _ = fixture.vm.events.send(VmEvent::Log {
        stream: vm_trait::LogStream::Stdout,
        bytes: b"flush deadline".to_vec(),
    });
    let flush_started = std::time::Instant::now();
    fixture.control.request_drain();
    timeout(Duration::from_secs(1), store.append_started.notified())
        .await
        .expect("blocked append");
    assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
    let mut task = fixture.task;
    assert!(
        timeout(Duration::from_millis(500), &mut task)
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .ownership
            .events
            .lock()
            .expect("ownership events")
            .last()
            .copied(),
        Some("stopping")
    );
    assert!(
        timeout(Duration::from_secs(4), &mut task)
            .await
            .expect("bounded final flush")
            .expect("coordinator join")
            .is_ok()
    );
    assert!(flush_started.elapsed() >= Duration::from_millis(1_500));
    assert_eq!(
        fixture
            .ownership
            .events
            .lock()
            .expect("ownership events")
            .last()
            .copied(),
        Some("cleaned")
    );
}

#[tokio::test]
// Keep the fixture-owned VM and coordinator control alive until the
// joined task has completed its durable cleanup transition.
#[allow(clippy::significant_drop_tightening)]
async fn stale_log_lease_stops_appends_without_rebinding_fence() {
    let store = Arc::new(LifecycleLogStore {
        append_started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        records: AtomicUsize::new(0),
        responses: Mutex::new(VecDeque::from([Err(
            crate::GatewayServiceLogStoreError::StaleLease,
        )])),
        leases: Mutex::new(Vec::new()),
        events: Mutex::new(Vec::new()),
    });
    let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
    let fixture = start_drain_fixture_with_logs(
        default_count_state(),
        false,
        false,
        Duration::from_secs(2),
        true,
        Some(store_port),
        Duration::from_secs(10),
    )
    .await;
    let _ = fixture.vm.events.send(VmEvent::Log {
        stream: vm_trait::LogStream::Stdout,
        bytes: b"stale lease".to_vec(),
    });
    timeout(Duration::from_secs(1), store.append_started.notified())
        .await
        .expect("stale append");
    tokio::task::yield_now().await;
    for _ in 0..3 {
        let _ = fixture.vm.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"after stale".to_vec(),
        });
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(store.calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.records.load(Ordering::Relaxed), 1);
    assert_eq!(
        store.leases.lock().expect("log store leases").as_slice(),
        &[(fixture.key.identity, fixture.key.fencing_token)]
    );
    fixture.control.request_drain();
    assert!(fixture.task.await.expect("coordinator join").is_ok());
}

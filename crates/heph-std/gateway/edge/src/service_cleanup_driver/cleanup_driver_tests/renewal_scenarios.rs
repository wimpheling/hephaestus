use super::*;
#[tokio::test]
async fn stale_retry_renewal_does_not_touch_physical_or_durable_state() {
    let (owner, lease, policy) = fixture();
    let provider = Arc::new(TestProvider {
        gate: None,
        started: None,
        calls: AtomicUsize::new(0),
    });
    let mut stale = lease.clone();
    stale.fencing_token += 1;
    let ownership = Arc::new(TestOwnership {
        renewals: AtomicUsize::new(0),
        mark_cleaned: AtomicUsize::new(0),
        renewal_lease: Mutex::new(stale),
        mark_result: Mutex::new(Ok(())),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let failures = Arc::new(TestFailures {
        calls: AtomicUsize::new(0),
        result: Mutex::new(Ok(())),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let targets = Arc::new(TestTargets {
        instance: Mutex::new(Some(lease.clone())),
        responses: Mutex::new(Vec::new()),
    });
    let driver = driver_parts(
        &owner,
        policy,
        Arc::clone(&provider),
        Arc::clone(&ownership),
        Arc::clone(&failures),
        targets,
    );
    let mut lease = lease;
    let error = driver
        .renew_and_prepare_stopping(&mut lease, Instant::now() + Duration::from_secs(1))
        .await
        .expect_err("stale renewal");
    assert_eq!(error, GatewayServiceCleanupDriverError::Stale);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
    assert_eq!(failures.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn confirmed_cleanup_accepts_exact_cleaned_row_without_renewal() {
    let (owner, lease, policy) = fixture();
    let provider = Arc::new(TestProvider {
        gate: None,
        started: None,
        calls: AtomicUsize::new(0),
    });
    let ownership = Arc::new(TestOwnership {
        renewals: AtomicUsize::new(0),
        mark_cleaned: AtomicUsize::new(0),
        renewal_lease: Mutex::new(lease.clone()),
        mark_result: Mutex::new(Ok(())),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let failures = Arc::new(TestFailures {
        calls: AtomicUsize::new(0),
        result: Mutex::new(Ok(())),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let mut cleaned = lease.clone();
    cleaned.state = GatewayServiceInstanceState::Cleaned;
    let targets = Arc::new(TestTargets {
        instance: Mutex::new(Some(cleaned)),
        responses: Mutex::new(Vec::new()),
    });
    let driver = driver_parts(
        &owner,
        policy,
        Arc::clone(&provider),
        Arc::clone(&ownership),
        Arc::clone(&failures),
        targets,
    );
    let cleanup =
        GatewayServiceCleanup::from_confirmed_physical(lease.identity, Duration::from_secs(1))
            .expect("confirmed cleanup");
    assert!(
        driver
            .confirm_cleaned_state(&cleanup, &lease)
            .await
            .expect("cleaned confirmation")
    );
    assert_eq!(ownership.renewals.load(Ordering::Relaxed), 0);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn heartbeat_continues_while_physical_cleanup_is_blocked() {
    let (owner, lease, policy) = fixture();
    let gate = Arc::new(Notify::new());
    let started = Arc::new(Notify::new());
    let provider = Arc::new(TestProvider {
        gate: Some(Arc::clone(&gate)),
        started: Some(Arc::clone(&started)),
        calls: AtomicUsize::new(0),
    });
    let ownership = Arc::new(TestOwnership {
        renewals: AtomicUsize::new(0),
        mark_cleaned: AtomicUsize::new(0),
        renewal_lease: Mutex::new(lease.clone()),
        mark_result: Mutex::new(Ok(())),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let failures = Arc::new(TestFailures {
        calls: AtomicUsize::new(0),
        result: Mutex::new(Ok(())),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let targets = Arc::new(TestTargets {
        instance: Mutex::new(Some(lease.clone())),
        responses: Mutex::new(Vec::new()),
    });
    let driver = driver_parts(
        &owner,
        policy,
        Arc::clone(&provider),
        ownership.clone(),
        failures,
        targets,
    );
    let mut cleanup =
        GatewayServiceCleanup::new(lease.identity, None, Duration::from_secs(1)).expect("cleanup");
    let mut lease = lease;
    let mut pending = None;
    let task = tokio::spawn(async move {
        driver
            .attempt(
                &mut cleanup,
                &mut lease,
                &mut pending,
                Instant::now() + Duration::from_secs(2),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), started.notified())
        .await
        .expect("physical cleanup started");
    let renewals_before_blocked_cleanup = ownership.renewals.load(Ordering::Relaxed);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if ownership.renewals.load(Ordering::Relaxed) > renewals_before_blocked_cleanup {
                break;
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("heartbeat during blocked physical cleanup");
    gate.notify_one();
    assert_eq!(
        task.await.expect("driver task").expect("driver result"),
        GatewayServiceCleanupDriverOutcome::Cleaned
    );
}

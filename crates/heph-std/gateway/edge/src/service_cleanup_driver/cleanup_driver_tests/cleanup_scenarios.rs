use super::*;
#[tokio::test]
async fn expired_deadline_confirms_cleaned_physical_state_without_renewal() {
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
        provider,
        ownership.clone(),
        failures,
        targets,
    );
    let mut cleanup =
        GatewayServiceCleanup::from_confirmed_physical(lease.identity, Duration::from_secs(1))
            .expect("cleanup");
    let mut lease = lease;
    let mut pending = None;
    assert_eq!(
        driver
            .attempt(
                &mut cleanup,
                &mut lease,
                &mut pending,
                Instant::now() - Duration::from_millis(1)
            )
            .await,
        Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
    );
    assert_eq!(ownership.renewals.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn lease_loss_still_settles_physical_cleanup_and_retains_state() {
    let (owner, lease, mut policy) = fixture();
    policy.lease.renewal_interval = Duration::from_millis(100);
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
        provider,
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
                Instant::now() + Duration::from_millis(50),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), started.notified())
        .await
        .expect("physical cleanup started");
    tokio::time::sleep(Duration::from_millis(100)).await;
    gate.notify_one();
    assert_eq!(
        task.await.expect("driver task").expect("driver result"),
        GatewayServiceCleanupDriverOutcome::Pending {
            physical_complete: true,
            failure_recorded: false,
            lease_lost: true
        }
    );
    assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn newer_fence_does_not_allow_stale_cleanup_write() {
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
        mark_result: Mutex::new(Err(GatewayServiceOwnershipError::StaleLease)),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let failures = Arc::new(TestFailures {
        calls: AtomicUsize::new(0),
        result: Mutex::new(Ok(())),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let mut newer = lease.clone();
    newer.fencing_token += 1;
    let targets = Arc::new(TestTargets {
        instance: Mutex::new(Some(newer)),
        responses: Mutex::new(Vec::new()),
    });
    let driver = driver_parts(
        &owner,
        policy,
        provider,
        ownership.clone(),
        failures,
        targets,
    );
    let mut cleanup =
        GatewayServiceCleanup::new(lease.identity, None, Duration::from_secs(1)).expect("cleanup");
    let mut lease = lease;
    let mut pending = None;
    assert_eq!(
        driver
            .attempt(
                &mut cleanup,
                &mut lease,
                &mut pending,
                Instant::now() + Duration::from_secs(2)
            )
            .await,
        Err(GatewayServiceCleanupDriverError::Stale)
    );
    assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn expired_cleaned_row_with_newer_fence_is_stale() {
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
    let mut newer = lease.clone();
    newer.fencing_token += 1;
    newer.state = GatewayServiceInstanceState::Cleaned;
    let targets = Arc::new(TestTargets {
        instance: Mutex::new(Some(newer)),
        responses: Mutex::new(Vec::new()),
    });
    let driver = driver_parts(
        &owner,
        policy,
        provider,
        ownership.clone(),
        failures,
        targets,
    );
    let mut cleanup =
        GatewayServiceCleanup::from_confirmed_physical(lease.identity, Duration::from_secs(1))
            .expect("cleanup");
    let mut lease = lease;
    let mut pending = None;
    assert_eq!(
        driver
            .attempt(
                &mut cleanup,
                &mut lease,
                &mut pending,
                Instant::now() - Duration::from_millis(1)
            )
            .await,
        Err(GatewayServiceCleanupDriverError::Stale)
    );
    assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
}

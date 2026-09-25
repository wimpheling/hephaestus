use super::*;
#[tokio::test]
async fn unavailable_failure_report_preserves_pending_and_blocks_cleaned() {
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
        result: Mutex::new(Err(GatewayServiceFailureStoreError::Unavailable)),
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
    let failure = GatewayServiceFailure::new(GatewayServiceFailureCode::Cleanup, None, None)
        .expect("failure");
    let mut pending = Some(failure);
    assert_eq!(
        driver
            .attempt(
                &mut cleanup,
                &mut lease,
                &mut pending,
                Instant::now() + Duration::from_secs(2)
            )
            .await,
        Err(GatewayServiceCleanupDriverError::Unavailable)
    );
    assert_eq!(pending, Some(failure));
    assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn successful_failure_report_precedes_cleaned_transition() {
    let (owner, lease, policy) = fixture();
    let events = Arc::new(Mutex::new(Vec::new()));
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
        events: Arc::clone(&events),
    });
    let failures = Arc::new(TestFailures {
        calls: AtomicUsize::new(0),
        result: Mutex::new(Ok(())),
        events: Arc::clone(&events),
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
        failures.clone(),
        targets,
    );
    let mut cleanup =
        GatewayServiceCleanup::new(lease.identity, None, Duration::from_secs(1)).expect("cleanup");
    let mut lease = lease;
    let mut pending = Some(
        GatewayServiceFailure::new(GatewayServiceFailureCode::Readiness, None, None)
            .expect("failure"),
    );
    assert_eq!(
        driver
            .attempt(
                &mut cleanup,
                &mut lease,
                &mut pending,
                Instant::now() + Duration::from_secs(2)
            )
            .await,
        Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
    );
    assert!(pending.is_none());
    assert_eq!(failures.calls.load(Ordering::Relaxed), 1);
    assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 1);
    assert_eq!(
        *events.lock().expect("events"),
        vec!["record_failure", "mark_cleaned"]
    );
}

#[tokio::test]
async fn ambiguous_cleaned_response_is_confirmed_without_repeating_physical_cleanup() {
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
        mark_result: Mutex::new(Err(GatewayServiceOwnershipError::Unavailable)),
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
        instance: Mutex::new(Some(cleaned.clone())),
        responses: Mutex::new(vec![Some(cleaned), Some(lease.clone())]),
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
                Instant::now() + Duration::from_secs(2)
            )
            .await,
        Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
    );
    assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 1);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
}

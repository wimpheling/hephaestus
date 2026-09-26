use super::*;
// The supervisor retains the cleanup future while this test observes a later
// claim-resolution poll.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test]
async fn poll_services_resolution_while_cleanup_is_pending() {
    let ownership = Arc::new(Noop::default());
    *ownership.claim_result.lock().expect("claim result") =
        Some(Err(GatewayServiceOwnershipError::Unavailable));
    let mut supervisor = supervisor_with(ownership);
    let request = request(Uuid::new_v4(), Uuid::new_v4());
    supervisor.start(request).expect("reservation");
    let uncertain = supervisor.poll().await.expect("uncertain claim");
    supervisor
        .cleanup_jobs
        .push(Box::pin(std::future::pending::<CleanupCompletion>()));
    let resolver = Arc::new(FixedClaimResolution {
        result: Mutex::new(Some(Ok(None))),
        calls: AtomicUsize::new(0),
    });
    supervisor
        .reconcile_claim(uncertain.job_id, resolver)
        .expect("resolution scheduling");
    let event = tokio::time::timeout(std::time::Duration::from_millis(100), supervisor.poll())
        .await
        .expect("resolution should not be starved")
        .expect("resolution event");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Settled);
    assert!(event.capacity_released);
    drop(supervisor);
}

// The supervisor owns the blocked resolution future; this test drops only
// the poll wait, then consumes the supervisor after releasing that future.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test]
async fn dropping_resolution_poll_wait_keeps_future_for_shutdown() {
    let ownership = Arc::new(Noop::default());
    *ownership.claim_result.lock().expect("claim result") =
        Some(Err(GatewayServiceOwnershipError::Unavailable));
    let mut supervisor = supervisor_with(ownership);
    let request = request(Uuid::new_v4(), Uuid::new_v4());
    supervisor.start(request).expect("reservation");
    let uncertain = supervisor.poll().await.expect("uncertain claim");
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    supervisor
        .reconcile_claim(
            uncertain.job_id,
            Arc::new(BlockingClaimResolution {
                started: Arc::clone(&started),
                release: Arc::clone(&release),
            }),
        )
        .expect("resolution scheduling");
    {
        let poll = supervisor.poll();
        tokio::pin!(poll);
        tokio::select! {
            () = started.notified() => {}
            _ = &mut poll => panic!("resolution should remain blocked"),
        }
    }
    release.notify_one();
    let event = supervisor.poll().await.expect("resolution result");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Uncertain);
    let shutdown = supervisor.shutdown().await;
    assert_eq!(shutdown.unresolved.len(), 1);
    assert_eq!(shutdown.unresolved[0].request, request);
}

#[tokio::test]
async fn exact_resolved_owned_claim_is_cleaned_before_capacity_release() {
    let (mut supervisor, provider, _ownership, request, _accepted) = ready_supervisor(false);
    let lease = supervisor
        .context
        .targets
        .get_service_instance(GatewayServiceIdentity {
            instance_id: Uuid::new_v4(),
            gateway_id: request.gateway_id,
            revision_id: request.revision_id,
        })
        .await
        .expect("target lookup")
        .expect("ready fixture lease");
    let token = supervisor
        .capacity
        .lock()
        .expect("capacity")
        .reserve(request.gateway_id, request.revision_id)
        .expect("capacity reservation");
    let id = Uuid::new_v4();
    let (status, _) = watch::channel(GatewayServiceSupervisorJobStatus::Uncertain);
    supervisor.records.insert(
        id,
        JobRecord {
            request,
            token,
            cancellation: CancellationToken::new(),
            status,
            capacity_retained: true,
            completion: Some(JobTerminal {
                lease: Some(lease.clone()),
                claim_uncertain: false,
                coordinator_failure: None,
                claim_cleanup_reason: Some(GatewayServiceCoordinatorFailureReason::Cancelled),
            }),
            cleanup_retry: None,
            cleanup_in_flight: false,
            claim_resolution_in_flight: false,
        },
    );
    let resolver = Arc::new(FixedClaimResolution {
        result: Mutex::new(Some(Ok(Some(lease)))),
        calls: AtomicUsize::new(0),
    });
    supervisor
        .reconcile_claim(id, resolver.clone())
        .expect("resolution scheduling");
    let pending = supervisor.poll().await.expect("cleanup scheduled");
    assert_eq!(
        pending.status,
        GatewayServiceSupervisorJobStatus::CleanupPending
    );
    assert_eq!(
        supervisor.reconcile_claim(id, resolver),
        Err(GatewayServiceSupervisorError::RetryAlreadyInFlight)
    );
    let cleaned = supervisor.poll().await.expect("cleanup result");
    assert_eq!(cleaned.status, GatewayServiceSupervisorJobStatus::Settled);
    assert!(cleaned.capacity_released);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 1);
    drop(supervisor);
}

#[tokio::test]
async fn definite_claim_conflict_releases_capacity() {
    let ownership = Arc::new(Noop::default());
    *ownership.claim_result.lock().expect("claim result") =
        Some(Err(GatewayServiceOwnershipError::Conflict));
    let mut supervisor = supervisor_with(Arc::clone(&ownership));
    let request = request(Uuid::new_v4(), Uuid::new_v4());
    supervisor.start(request).expect("reservation");
    let event = supervisor.poll().await.expect("conflict job");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Failed);
    assert!(event.capacity_released);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    drop(supervisor);
}

#[tokio::test]
async fn dropping_poll_wait_keeps_parent_owned_future() {
    let notify = Arc::new(Notify::new());
    let waiter = Arc::clone(&notify);
    let mut jobs: Vec<Pin<Box<dyn Future<Output = JobCompletion> + Send>>> =
        vec![Box::pin(async move {
            waiter.notified().await;
            JobCompletion {
                id: Uuid::new_v4(),
                status: GatewayServiceSupervisorJobStatus::Settled,
                capacity_released: true,
                terminal: JobTerminal {
                    lease: None,
                    claim_uncertain: false,
                    coordinator_failure: None,
                    claim_cleanup_reason: None,
                },
            }
        })];
    {
        let pending = super::super::poll_futures::poll_next_job(&mut jobs);
        tokio::pin!(pending);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut pending)
                .await
                .is_err()
        );
    }
    assert_eq!(jobs.len(), 1);
    notify.notify_one();
    assert!(
        super::super::poll_futures::poll_next_job(&mut jobs)
            .await
            .is_some()
    );
    assert!(jobs.is_empty());
}

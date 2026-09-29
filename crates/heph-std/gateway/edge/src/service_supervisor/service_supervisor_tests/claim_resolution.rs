use super::*;
// The supervisor owns the unresolved claim while this test drives
// reconciliation through its retained future.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test]
async fn unavailable_claim_is_quarantined_until_reconciliation() {
    let ownership = Arc::new(Noop::default());
    *ownership.claim_result.lock().expect("claim result") =
        Some(Err(GatewayServiceOwnershipError::Unavailable));
    let mut supervisor = supervisor_with(Arc::clone(&ownership));
    let request = request(Uuid::new_v4(), Uuid::new_v4());
    supervisor.start(request).expect("reservation");
    let event = supervisor.poll().await.expect("uncertain job");
    assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Uncertain);
    assert!(!event.capacity_released);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
    assert_eq!(supervisor.capacity_snapshot().starting_instances, 0);
    assert_eq!(
        supervisor.start(request).unwrap_err(),
        GatewayServiceSupervisorError::Duplicate
    );
    let shutdown = supervisor.shutdown().await;
    assert_eq!(shutdown.unresolved.len(), 1);
    assert_eq!(shutdown.unresolved[0].request, request);
}

#[tokio::test]
async fn serialized_absence_releases_an_uncertain_claim_reservation() {
    let ownership = Arc::new(Noop::default());
    *ownership.claim_result.lock().expect("claim result") =
        Some(Err(GatewayServiceOwnershipError::Unavailable));
    let mut supervisor = supervisor_with(ownership);
    let request = request(Uuid::new_v4(), Uuid::new_v4());
    supervisor.start(request).expect("reservation");
    let uncertain = supervisor.poll().await.expect("uncertain claim");
    assert_eq!(
        uncertain.status,
        GatewayServiceSupervisorJobStatus::Uncertain
    );
    let resolver = Arc::new(FixedClaimResolution {
        result: Mutex::new(Some(Ok(None))),
        calls: AtomicUsize::new(0),
    });
    supervisor
        .reconcile_claim(uncertain.job_id, resolver.clone())
        .expect("resolution scheduling");
    let resolution_event = supervisor.poll().await.expect("resolution result");
    assert_eq!(
        resolution_event.status,
        GatewayServiceSupervisorJobStatus::Settled
    );
    assert!(resolution_event.capacity_released);
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    drop(supervisor);
}

#[tokio::test]
async fn foreign_resolved_claim_remains_retained_without_cleanup() {
    let ownership = Arc::new(Noop::default());
    *ownership.claim_result.lock().expect("claim result") =
        Some(Err(GatewayServiceOwnershipError::Unavailable));
    let mut supervisor = supervisor_with(Arc::clone(&ownership));
    let request = request(Uuid::new_v4(), Uuid::new_v4());
    supervisor.start(request).expect("reservation");
    let uncertain = supervisor.poll().await.expect("uncertain claim");
    let owner = GatewayServiceOwner::new("foreign-host", Uuid::new_v4()).expect("owner");
    let foreign = lease(request, &owner);
    let resolver = Arc::new(FixedClaimResolution {
        result: Mutex::new(Some(Ok(Some(foreign)))),
        calls: AtomicUsize::new(0),
    });
    supervisor
        .reconcile_claim(uncertain.job_id, resolver.clone())
        .expect("resolution scheduling");
    let resolution_event = supervisor.poll().await.expect("resolution result");
    assert_eq!(
        resolution_event.status,
        GatewayServiceSupervisorJobStatus::Uncertain
    );
    assert!(!resolution_event.capacity_released);
    assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
    assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 1);
    supervisor
        .reconcile_claim(resolution_event.job_id, resolver)
        .expect("repeat resolution scheduling");
    let repeated_event = supervisor.poll().await.expect("repeat resolution result");
    assert_eq!(
        repeated_event.status,
        GatewayServiceSupervisorJobStatus::Uncertain
    );
    assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 1);
    drop(supervisor);
}

#[tokio::test]
async fn known_claim_resolution_retries_preserve_original_lease() {
    #[derive(Clone, Copy)]
    enum ResolutionCase {
        NewerFence,
        WrongInstance,
        WrongVmId,
        Expired,
        Unavailable,
    }

    for case in [
        ResolutionCase::NewerFence,
        ResolutionCase::WrongInstance,
        ResolutionCase::WrongVmId,
        ResolutionCase::Expired,
        ResolutionCase::Unavailable,
    ] {
        let (mut supervisor, provider, _ownership, request, _accepted) = ready_supervisor(false);
        let known = supervisor
            .context
            .targets
            .get_service_instance(GatewayServiceIdentity {
                instance_id: Uuid::nil(),
                gateway_id: request.gateway_id,
                revision_id: request.revision_id,
            })
            .await
            .expect("known target lookup")
            .expect("known lease");
        let result = match case {
            ResolutionCase::NewerFence => {
                let mut candidate = known.clone();
                candidate.fencing_token += 1;
                Ok(Some(candidate))
            }
            ResolutionCase::WrongInstance => {
                let mut candidate = known.clone();
                candidate.identity.instance_id = Uuid::new_v4();
                candidate.vm_id = format!("gateway-service-{}", candidate.identity.instance_id);
                Ok(Some(candidate))
            }
            ResolutionCase::WrongVmId => {
                let mut candidate = known.clone();
                candidate.vm_id = String::from("gateway-service-wrong");
                Ok(Some(candidate))
            }
            ResolutionCase::Expired => {
                let mut candidate = known.clone();
                candidate.lease_expires_at = OffsetDateTime::now_utc() - TimeDuration::seconds(1);
                Ok(Some(candidate))
            }
            ResolutionCase::Unavailable => Err(GatewayServiceOwnershipError::Unavailable),
        };
        let resolver = Arc::new(FixedClaimResolution {
            result: Mutex::new(Some(result)),
            calls: AtomicUsize::new(0),
        });
        let id = insert_known_claim_record(&mut supervisor, request, &known);
        for _ in 0..2 {
            supervisor
                .reconcile_claim(id, resolver.clone())
                .expect("resolution scheduling");
            let event = supervisor.poll().await.expect("resolution result");
            assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Uncertain);
            assert!(!event.capacity_released);
        }
        let record = supervisor.records.get(&id).expect("retained record");
        assert_eq!(
            record
                .completion
                .as_ref()
                .and_then(|terminal| terminal.lease.as_ref()),
            Some(&known)
        );
        assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
        drop(supervisor);
    }
}

// This test intentionally drops the supervisor after proving a ready
// resolution is not starved by an unrelated pending cleanup future.

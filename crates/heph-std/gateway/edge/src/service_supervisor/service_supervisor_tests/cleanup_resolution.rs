use super::*;
#[tokio::test]
async fn expired_cleanup_keeps_state_for_absent_foreign_newer_or_rolled_back_resolution() {
    #[derive(Clone, Copy)]
    enum ResolutionCase {
        Absent,
        Foreign,
        Newer,
        RolledBack,
    }

    for case in [
        ResolutionCase::Absent,
        ResolutionCase::Foreign,
        ResolutionCase::Newer,
        ResolutionCase::RolledBack,
    ] {
        let (mut supervisor, provider, ownership, initial_request, _accepted) =
            ready_supervisor(true);
        let handle = supervisor.start(initial_request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        handle.cancel();
        let first = supervisor.poll().await.expect("initial cleanup result");
        assert!(!first.capacity_released);
        provider.fail_destroy.store(false, Ordering::Relaxed);
        ownership.renew_stale.store(true, Ordering::Relaxed);

        let old = supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.clone())
            .expect("retained lease");
        let resolved = match case {
            ResolutionCase::Absent => Ok(None),
            ResolutionCase::Foreign => {
                let mut candidate = old.clone();
                candidate.owner_host_id = String::from("other-host");
                candidate.owner_uuid = Uuid::new_v4();
                Ok(Some(candidate))
            }
            ResolutionCase::Newer => {
                let mut candidate = old.clone();
                candidate.fencing_token += 2;
                Ok(Some(candidate))
            }
            ResolutionCase::RolledBack => Ok(Some(old.clone())),
        };
        let recovery = Arc::new(FixedExpiredRecovery {
            ownership: Arc::clone(&ownership),
            takeover: Mutex::new(VecDeque::from([Err(
                GatewayServiceOwnershipError::Unavailable,
            )])),
            resolution: Mutex::new(VecDeque::from([resolved])),
            takeover_calls: AtomicUsize::new(0),
            resolution_calls: AtomicUsize::new(0),
        });
        supervisor
            .retry_cleanup_with_recovery(first.job_id, recovery)
            .expect("recovery retry");
        let rejected = supervisor.poll().await.expect("recovery result");
        assert_eq!(rejected.status, GatewayServiceSupervisorJobStatus::Failed);
        assert!(!rejected.capacity_released);
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            supervisor
                .records
                .get(&first.job_id)
                .and_then(|record| record.completion.as_ref())
                .and_then(|completion| completion.lease.as_ref()),
            Some(&old)
        );
    }
}

#[tokio::test]
async fn blocked_cleanup_retry_does_not_stop_other_startup_jobs() {
    let (mut supervisor, provider, _ownership, initial_request, _accepted) = ready_supervisor(true);
    let handle = supervisor.start(initial_request).expect("reservation");
    wait_until_ready(&mut supervisor, &handle).await;
    handle.cancel();
    let first = supervisor.poll().await.expect("initial cleanup result");
    assert!(!first.capacity_released);
    provider.fail_destroy.store(false, Ordering::Relaxed);
    let gate = Arc::new(Notify::new());
    let started = Arc::new(Notify::new());
    *provider.destroy_gate.lock().expect("destroy gate") = Some(Arc::clone(&gate));
    *provider.destroy_started.lock().expect("destroy started") = Some(Arc::clone(&started));
    supervisor
        .retry_cleanup(first.job_id)
        .expect("retry scheduling");
    {
        let poll = supervisor.poll();
        tokio::pin!(poll);
        tokio::select! {
            () = started.notified() => {}
            event = &mut poll => panic!("cleanup ended before blocking: {event:?}"),
        }
    }

    let other_request = request(Uuid::new_v4(), Uuid::new_v4());
    let other = supervisor.start(other_request).expect("other startup slot");
    let other_event = tokio::time::timeout(std::time::Duration::from_secs(1), supervisor.poll())
        .await
        .expect("other job remains schedulable")
        .expect("other job result");
    assert_eq!(other_event.job_id, other.job_id());
    assert_eq!(
        other_event.status,
        GatewayServiceSupervisorJobStatus::Failed
    );
    assert!(supervisor.has_pending_jobs());
    gate.notify_one();
    let cleanup_event = tokio::time::timeout(std::time::Duration::from_secs(1), supervisor.poll())
        .await
        .expect("cleanup retry settles")
        .expect("cleanup result");
    assert_eq!(cleanup_event.job_id, first.job_id);
    assert!(cleanup_event.capacity_released);
}

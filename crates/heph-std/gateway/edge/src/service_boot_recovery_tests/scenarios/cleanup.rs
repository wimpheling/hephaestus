use super::super::helpers::poll_cleanup_jobs;
use super::super::*;
use super::support_adapters::{TestProvider, TestTargets};
use super::support_context::{context_with_targets_and_provider, inventory_lease};
use super::support_ownership::TestOwnership;
use std::collections::HashSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use tokio::time::Duration;
use uuid::Uuid;

#[tokio::test]
async fn blocked_cleanup_does_not_starve_another_owned_slot() {
    let first = inventory_lease();
    let second = inventory_lease();
    let first_state = BootCleanupState {
        cleanup: GatewayServiceCleanup::new(first.identity, None, Duration::from_secs(1))
            .expect("cleanup"),
        lease: first.clone(),
        pending_failure: None,
    };
    let second_state = BootCleanupState {
        cleanup: GatewayServiceCleanup::new(second.identity, None, Duration::from_secs(1))
            .expect("cleanup"),
        lease: second.clone(),
        pending_failure: None,
    };
    let blocker = Box::pin(async move {
        std::future::pending::<()>().await;
        CleanupCompletion {
            state: first_state,
            result: Ok(GatewayServiceCleanupDriverOutcome::Pending {
                physical_complete: false,
                failure_recorded: false,
                lease_lost: false,
            }),
        }
    });
    let ready = Box::pin(async move {
        CleanupCompletion {
            state: second_state,
            result: Err(GatewayServiceCleanupDriverError::Unavailable),
        }
    });
    let mut jobs: Vec<CleanupFuture> = vec![blocker, ready];
    let completion = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        poll_cleanup_jobs(&mut jobs),
    )
    .await
    .expect("ready cleanup must win");
    assert!(completion.is_some());
    assert_eq!(jobs.len(), 1);
}

#[tokio::test]
async fn both_cleanup_slots_renew_while_physical_teardown_is_blocked() {
    let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
    let mut leases = Vec::new();
    for _ in 0..2 {
        let mut lease = inventory_lease();
        lease.owner_host_id = owner.host_id.clone();
        lease.owner_uuid = owner.owner_uuid;
        lease.state = GatewayServiceInstanceState::Stopping;
        leases.push(lease);
    }
    let batches = vec![leases.clone()];
    let cleaned = Arc::new(Mutex::new(HashSet::new()));
    let mut ownership = TestOwnership::batched_with_cleanup_state(batches, Arc::clone(&cleaned));
    ownership.renew_own = true;
    let ownership = Arc::new(ownership);
    let inventory = leases
        .iter()
        .cloned()
        .map(|mut lease| {
            lease.owner_uuid = Uuid::new_v4();
            lease.state = GatewayServiceInstanceState::Provisioning;
            lease
        })
        .collect();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let started_count = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(TestProvider {
        physical: None,
        block_started: Some(Arc::clone(&started)),
        block_release: Some(Arc::clone(&release)),
        block_count: Some(Arc::clone(&started_count)),
    });
    let targets = Arc::new(TestTargets {
        instances: Mutex::new(leases.clone()),
        inventory: Mutex::new(inventory),
        list_calls: AtomicUsize::new(0),
        hide_after: None,
        cleaned: Some(Arc::clone(&cleaned)),
    });
    let mut context =
        context_with_targets_and_provider(owner, Arc::clone(&ownership), targets, provider);
    context.cleanup_policy.lease.lease_duration = Duration::from_millis(300);
    context.cleanup_policy.lease.renewal_interval = Duration::from_millis(20);
    let mut recovery = GatewayServiceBootRecovery::new(context).expect("gate");
    assert_eq!(
        recovery.poll().await.expect("claim"),
        GatewayServiceBootRecoveryEvent::Pending
    );

    {
        let step = recovery.poll();
        tokio::pin!(step);
        tokio::select! {
            () = started.notified() => {}
            result = &mut step => panic!("cleanup should remain blocked: {result:?}"),
        }
        let mut renewed = false;
        for _ in 0..20 {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(20)) => {
                    if ownership.renew_calls.load(Ordering::Relaxed) >= 2 {
                        renewed = true;
                        break;
                    }
                }
                result = &mut step => panic!("cleanup completed before release: {result:?}"),
            }
        }
        assert!(renewed, "both blocked cleanups must continue lease renewal");
        let renewed_instances = ownership
            .renewed_instances
            .lock()
            .expect("renewed instances")
            .clone();
        assert_eq!(renewed_instances.len(), 2);
        assert!(
            leases
                .iter()
                .all(|lease| { renewed_instances.contains(&lease.identity.instance_id) })
        );
        release.notify_waiters();
        let event = tokio::time::timeout(Duration::from_secs(1), &mut step)
            .await
            .expect("cleanup settles")
            .expect("cleanup event");
        assert_eq!(event, GatewayServiceBootRecoveryEvent::Pending);
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), recovery.poll())
        .await
        .expect("second cleanup settles")
        .expect("second cleanup event");
    assert_eq!(cleaned.lock().expect("cleaned").len(), 2);
}

use super::super::*;
use super::support_adapters::{TestProvider, TestTargets};
use super::support_context::{
    context, context_with_owner, context_with_targets_and_provider, inventory_lease,
};
use super::support_ownership::TestOwnership;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use uuid::Uuid;

#[tokio::test]
async fn empty_first_inventory_page_proves_complete() {
    let mut recovery = GatewayServiceBootRecovery::new(context(Vec::new())).expect("gate");
    assert_eq!(
        recovery.poll().await.expect("empty proof"),
        GatewayServiceBootRecoveryEvent::Complete
    );
    assert!(recovery.is_complete());
}

#[tokio::test]
async fn unexpired_inventory_keeps_gate_closed() {
    let mut recovery =
        GatewayServiceBootRecovery::new(context(vec![inventory_lease()])).expect("gate");
    assert_eq!(
        recovery.poll().await.expect("bounded wait"),
        GatewayServiceBootRecoveryEvent::Waiting
    );
    assert!(!recovery.is_complete());
    assert!(!recovery.has_pending_work());
}

#[tokio::test]
async fn own_stopping_inventory_is_renewed_before_cleanup() {
    let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
    let ownership = Arc::new(TestOwnership::renew_own());
    let mut lease = inventory_lease();
    lease.owner_host_id = owner.host_id.clone();
    lease.owner_uuid = owner.owner_uuid;
    lease.state = GatewayServiceInstanceState::Stopping;
    let identity = lease.identity;
    let mut recovery = GatewayServiceBootRecovery::new(context_with_owner(
        vec![lease],
        owner,
        Arc::clone(&ownership),
    ))
    .expect("gate");

    assert_eq!(
        recovery.poll().await.expect("renew own row"),
        GatewayServiceBootRecoveryEvent::Pending
    );
    assert_eq!(ownership.renew_calls.load(Ordering::Relaxed), 1);
    let shutdown = recovery.shutdown().await;
    assert_eq!(shutdown.unresolved.len(), 1);
    assert_eq!(shutdown.unresolved[0].lease.identity, identity);
}

#[tokio::test]
async fn partial_renewal_failure_keeps_successful_claim_bounded_and_owned() {
    let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
    let mut leases = Vec::new();
    for _ in 0..2 {
        let mut lease = inventory_lease();
        lease.owner_host_id = owner.host_id.clone();
        lease.owner_uuid = owner.owner_uuid;
        lease.state = GatewayServiceInstanceState::Stopping;
        leases.push(lease);
    }
    let mut ownership = TestOwnership::renew_own();
    ownership.renew_fail_after = Some(1);
    let ownership = Arc::new(ownership);
    let mut recovery =
        GatewayServiceBootRecovery::new(context_with_owner(leases, owner, Arc::clone(&ownership)))
            .expect("gate");

    assert_eq!(
        recovery.poll().await,
        Err(GatewayServiceBootRecoveryError::Unavailable)
    );
    assert!(recovery.unresolved_claims.is_empty());
    assert_eq!(recovery.cleanup_jobs.len(), 1);
    assert!(!recovery.is_complete());
    let shutdown = recovery.shutdown().await;
    assert!(shutdown.unresolved_claims.is_empty());
    assert_eq!(shutdown.unresolved.len(), 1);
}

#[tokio::test]
async fn backlog_is_drained_in_two_claim_slots_before_fresh_empty_proof() {
    let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
    let mut leases = Vec::new();
    for _ in 0..12 {
        let mut lease = inventory_lease();
        lease.owner_host_id = owner.host_id.clone();
        lease.owner_uuid = owner.owner_uuid;
        lease.state = GatewayServiceInstanceState::Stopping;
        leases.push(lease);
    }
    let batches = leases.chunks(2).map(<[_]>::to_vec).collect::<Vec<_>>();
    let cleaned = Arc::new(Mutex::new(HashSet::new()));
    let inventory = leases
        .iter()
        .cloned()
        .map(|mut lease| {
            lease.owner_uuid = Uuid::new_v4();
            lease.state = GatewayServiceInstanceState::Provisioning;
            lease
        })
        .collect();
    let ownership = Arc::new(TestOwnership::batched_with_cleanup_state(
        batches,
        Arc::clone(&cleaned),
    ));
    let physical = Arc::new(Mutex::new(HashSet::new()));
    let targets = Arc::new(TestTargets {
        instances: Mutex::new(leases.clone()),
        inventory: Mutex::new(inventory),
        list_calls: AtomicUsize::new(0),
        hide_after: None,
        cleaned: Some(Arc::clone(&cleaned)),
    });
    let mut recovery = GatewayServiceBootRecovery::new(context_with_targets_and_provider(
        owner,
        Arc::clone(&ownership),
        targets,
        Arc::new(TestProvider {
            physical: Some(Arc::clone(&physical)),
            ..TestProvider::standard()
        }),
    ))
    .expect("gate");

    let mut completed = false;
    for _ in 0..20 {
        if tokio::time::timeout(std::time::Duration::from_secs(3), recovery.poll())
            .await
            .expect("bounded boot step")
            .expect("recovery step")
            == GatewayServiceBootRecoveryEvent::Complete
        {
            completed = true;
            break;
        }
    }
    assert!(completed, "backlog must reach a fresh empty proof");
    assert_eq!(cleaned.lock().expect("cleaned").len(), 12);
    assert_eq!(physical.lock().expect("physical").len(), 12);
    assert!(
        ownership
            .claim_limits
            .lock()
            .expect("claim limits")
            .iter()
            .all(|limit| *limit <= MAX_SERVICE_BOOT_RECOVERY_CLEANUPS)
    );
    assert_eq!(
        ownership.claim_limits.lock().expect("claim limits").len(),
        6
    );
}

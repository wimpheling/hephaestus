use super::super::helpers::{valid_exact_takeover_lease, valid_owned_recovery_lease};
use super::super::*;
use super::support_context::{context, context_with_owner, inventory_lease};
use super::support_ownership::TestOwnership;
use tokio::sync::Notify;
use tokio::time::Instant;
use uuid::Uuid;

#[tokio::test]
async fn dropped_poll_keeps_claim_owned_until_shutdown() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let ownership = Arc::new(TestOwnership::blocking(
        Arc::clone(&started),
        Arc::clone(&release),
    ));
    let mut recovery = GatewayServiceBootRecovery::new(context_with_owner(
        vec![inventory_lease()],
        GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner"),
        ownership,
    ))
    .expect("gate");
    {
        let poll = recovery.poll();
        tokio::pin!(poll);
        tokio::select! {
            _ = &mut poll => panic!("claim should remain parent-owned"),
            () = started.notified() => {}
        }
    }
    assert!(recovery.has_pending_work());
    release.notify_waiters();
    let shutdown = tokio::time::timeout(std::time::Duration::from_secs(1), recovery.shutdown())
        .await
        .expect("shutdown settles claim");
    assert!(shutdown.unresolved.is_empty());
    assert!(shutdown.unresolved_claims.is_empty());
}

#[tokio::test]
async fn malformed_batch_claim_is_retained_for_later_resolution() {
    let mut recovery = GatewayServiceBootRecovery::new(context(Vec::new())).expect("gate");
    let malformed = inventory_lease();
    let identity = malformed.identity;
    recovery
        .finish_claim(ClaimCompletion {
            result: Ok(vec![malformed]),
            deadline: Instant::now(),
            known_leases: Vec::new(),
        })
        .expect_err("malformed claim must close the gate");
    let shutdown = recovery.shutdown().await;
    assert_eq!(shutdown.unresolved_claims.len(), 1);
    assert_eq!(shutdown.unresolved_claims[0].identity, identity);
}

#[tokio::test]
async fn malformed_batch_blocks_fresh_empty_completion_and_keeps_all_claims() {
    let context = context(Vec::new());
    let mut valid = inventory_lease();
    valid.owner_host_id = context.owner.host_id.clone();
    valid.owner_uuid = context.owner.owner_uuid;
    valid.state = GatewayServiceInstanceState::Stopping;
    let malformed = inventory_lease();
    let mut recovery = GatewayServiceBootRecovery::new(context).expect("gate");
    recovery
        .finish_claim(ClaimCompletion {
            result: Ok(vec![malformed, valid]),
            deadline: Instant::now(),
            known_leases: Vec::new(),
        })
        .expect_err("mixed malformed batch must close the gate");
    assert_eq!(
        recovery.poll().await.expect("closed gate"),
        GatewayServiceBootRecoveryEvent::Waiting
    );
    assert!(!recovery.is_complete());
    let shutdown = recovery.shutdown().await;
    assert_eq!(shutdown.unresolved_claims.len(), 2);
}

#[test]
fn exact_retry_rejects_a_different_identity_or_fence() {
    let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
    let mut expected = inventory_lease();
    expected.owner_host_id = owner.host_id.clone();
    expected.owner_uuid = owner.owner_uuid;
    expected.state = GatewayServiceInstanceState::Stopping;
    let mut returned = expected.clone();
    assert!(valid_owned_recovery_lease(&returned, &owner, &expected));
    assert!(!valid_exact_takeover_lease(&returned, &owner, &expected));
    returned.fencing_token = expected.fencing_token + 1;
    assert!(valid_exact_takeover_lease(&returned, &owner, &expected));
    returned.identity.instance_id = Uuid::new_v4();
    returned.vm_id = format!("gateway-service-{}", returned.identity.instance_id);
    assert!(!valid_owned_recovery_lease(&returned, &owner, &expected));
}

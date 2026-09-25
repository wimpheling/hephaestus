use super::*;
use crate::{
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceOwner,
    GatewayServiceOwnership, GatewayServiceOwnershipError,
};
use async_trait::async_trait;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    sync::Notify,
    time::{Duration, Instant},
};
use uuid::Uuid;

enum RenewAction {
    Success {
        lease: GatewayServiceInstanceLease,
        delay: Duration,
    },
    Failure {
        error: GatewayServiceOwnershipError,
        delay: Duration,
    },
}

struct MockOwnership {
    renewals: Mutex<VecDeque<RenewAction>>,
    calls: AtomicUsize,
    started: Arc<Notify>,
    call_starts: Mutex<Vec<Instant>>,
}

impl MockOwnership {
    fn new(renewals: Vec<RenewAction>) -> Arc<Self> {
        Arc::new(Self {
            renewals: Mutex::new(renewals.into()),
            calls: AtomicUsize::new(0),
            started: Arc::new(Notify::new()),
            call_starts: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl GatewayServiceOwnership for MockOwnership {
    async fn claim_new(
        &self,
        _: Uuid,
        _: Uuid,
        _: &GatewayServiceOwner,
        _: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }

    async fn renew(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.call_starts
            .lock()
            .expect("call start mutex")
            .push(Instant::now());
        self.started.notify_waiters();
        let action = self
            .renewals
            .lock()
            .expect("renewal mutex")
            .pop_front()
            .unwrap_or(RenewAction::Failure {
                error: GatewayServiceOwnershipError::Unavailable,
                delay: Duration::ZERO,
            });
        match action {
            RenewAction::Success { lease, delay } => {
                tokio::time::sleep(delay).await;
                Ok(lease)
            }
            RenewAction::Failure { error, delay } => {
                tokio::time::sleep(delay).await;
                Err(error)
            }
        }
    }

    async fn claim_expired(
        &self,
        _: &GatewayServiceOwner,
        _: Duration,
        _: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }

    async fn mark_stopping(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }

    async fn mark_starting(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }

    async fn mark_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }

    async fn mark_draining(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }

    async fn promote_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }

    async fn mark_cleaned(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
}

fn owner() -> GatewayServiceOwner {
    GatewayServiceOwner::new("lease-test-host", Uuid::new_v4()).expect("owner")
}

fn lease(
    owner: &GatewayServiceOwner,
    state: GatewayServiceInstanceState,
) -> GatewayServiceInstanceLease {
    let now = ::time::OffsetDateTime::now_utc();
    let identity = crate::GatewayServiceIdentity {
        instance_id: Uuid::new_v4(),
        gateway_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    GatewayServiceInstanceLease {
        identity,
        owner_host_id: owner.host_id.clone(),
        owner_uuid: owner.owner_uuid,
        fencing_token: 4,
        state,
        vm_id: format!("gateway-service-{}", identity.instance_id),
        lease_expires_at: now + ::time::Duration::seconds(10),
        heartbeat_at: now,
    }
}

fn policy() -> GatewayServiceLeasePolicy {
    GatewayServiceLeasePolicy {
        lease_duration: Duration::from_millis(100),
        renewal_interval: Duration::from_millis(20),
    }
}

async fn assert_invalid_renewal(
    owner: GatewayServiceOwner,
    initial: GatewayServiceInstanceLease,
    returned: GatewayServiceInstanceLease,
) {
    let (monitor, control) = GatewayServiceLeaseMonitor::new(
        MockOwnership::new(vec![RenewAction::Success {
            lease: returned,
            delay: Duration::ZERO,
        }]),
        initial,
        owner,
        policy(),
        Instant::now() + Duration::from_millis(80),
    )
    .expect("monitor");
    let task = tokio::spawn(monitor.run());
    tokio::time::advance(Duration::from_millis(20)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        task.await.expect("monitor task"),
        GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Invalid)
    );
    drop(control);
}

#[path = "service_lease_tests/lifecycle.rs"]
mod lifecycle;
#[path = "service_lease_tests/policy_and_deadline.rs"]
mod policy_and_deadline;

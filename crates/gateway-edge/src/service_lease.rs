//! Caller-owned durable lease renewal for one persistent gateway service.

use std::{convert::TryFrom, sync::Arc, time::Duration};
use tokio::{
    sync::watch,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;

use crate::{
    GatewayServiceInstanceLease, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError, MAX_SERVICE_OWNERSHIP_LEASE,
};

/// Policy for one caller-owned service lease heartbeat monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceLeasePolicy {
    /// Duration requested from each durable renewal.
    pub lease_duration: Duration,
    /// Delay between renewal attempts while the lease remains valid.
    pub renewal_interval: Duration,
}

impl GatewayServiceLeasePolicy {
    /// Validates that renewals occur before the requested lease expires.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceLeaseError::InvalidPolicy`] for zero, overly
    /// long, or non renewing-before-expiry durations.
    pub fn validate(self) -> Result<(), GatewayServiceLeaseError> {
        if self.lease_duration.is_zero()
            || self.renewal_interval.is_zero()
            || self.renewal_interval >= self.lease_duration
            || self.lease_duration > MAX_SERVICE_OWNERSHIP_LEASE
        {
            return Err(GatewayServiceLeaseError::InvalidPolicy);
        }
        Ok(())
    }
}

/// Redacted reason a lease monitor can no longer renew its claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceLeaseLossReason {
    /// The local monotonic lease deadline elapsed.
    Expired,
    /// Durable ownership no longer matches this monitor.
    Stale,
    /// The durable claim cannot be renewed for its current lifecycle.
    Conflict,
    /// A malformed or invalid durable response was returned.
    Invalid,
}

/// A retryable heartbeat failure exposed without database details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceLeaseRetryReason {
    /// Durable storage was temporarily unavailable.
    Unavailable,
}

/// Current monitor state suitable for a parent lifecycle actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayServiceLeaseStatus {
    /// The exact claim remains usable until the monotonic deadline.
    Active {
        /// Most recent validated durable lease row.
        lease: GatewayServiceInstanceLease,
        /// Monotonic local deadline for the current lease.
        deadline: Instant,
        /// Last retryable failure, if one occurred after the last success.
        last_retry: Option<GatewayServiceLeaseRetryReason>,
    },
    /// Renewal is no longer safe and the parent must shut down the service.
    Lost(GatewayServiceLeaseLossReason),
    /// The monitor was explicitly stopped or its control handle was dropped.
    Stopped,
}

/// Control and status subscription for a lease monitor.
pub struct GatewayServiceLeaseControl {
    cancellation: CancellationToken,
    status: watch::Sender<GatewayServiceLeaseStatus>,
}

impl GatewayServiceLeaseControl {
    /// Stops the monitor and immediately marks its claim unavailable locally.
    pub fn stop(&self) {
        self.cancellation.cancel();
        mark_stopped(&self.status);
    }

    /// Subscribes to current lease and terminal status updates.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<GatewayServiceLeaseStatus> {
        self.status.subscribe()
    }
}

impl Drop for GatewayServiceLeaseControl {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Result of joining a caller-owned lease monitor future.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceLeaseRunResult {
    /// The caller stopped the monitor.
    Stopped,
    /// Durable ownership was lost and the parent must clean up the service.
    Lost(GatewayServiceLeaseLossReason),
}

/// Construction failures for a durable lease monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceLeaseError {
    /// Policy durations are zero or renew too slowly.
    #[error("invalid service lease monitor policy")]
    InvalidPolicy,
    /// The initial lease or owner identity is not internally consistent.
    #[error("invalid service lease monitor identity")]
    InvalidIdentity,
    /// The caller supplied an already elapsed initial deadline.
    #[error("service lease monitor deadline is already elapsed")]
    ElapsedDeadline,
}

/// A future that renews one exact durable service claim without spawning work.
pub struct GatewayServiceLeaseMonitor<O: ?Sized> {
    ownership: Arc<O>,
    lease: GatewayServiceInstanceLease,
    owner: GatewayServiceOwner,
    policy: GatewayServiceLeasePolicy,
    deadline: Instant,
    cancellation: CancellationToken,
    status: watch::Sender<GatewayServiceLeaseStatus>,
}

impl<O> GatewayServiceLeaseMonitor<O>
where
    O: GatewayServiceOwnership + ?Sized,
{
    /// Constructs a monitor from a claim and a deadline captured before claim.
    ///
    /// The caller must calculate `initial_deadline` before starting the claim
    /// operation. This prevents a slow claim from granting a new full local
    /// lease after it returns.
    ///
    /// # Errors
    ///
    /// Returns an error when policy, identity, or the supplied initial
    /// monotonic deadline is invalid.
    pub fn new(
        ownership: Arc<O>,
        lease: GatewayServiceInstanceLease,
        owner: GatewayServiceOwner,
        policy: GatewayServiceLeasePolicy,
        initial_deadline: Instant,
    ) -> Result<(Self, GatewayServiceLeaseControl), GatewayServiceLeaseError> {
        policy.validate()?;
        owner
            .validate()
            .map_err(|_| GatewayServiceLeaseError::InvalidIdentity)?;
        validate_lease(&lease, &lease, &owner)
            .map_err(|_| GatewayServiceLeaseError::InvalidIdentity)?;
        if initial_deadline <= Instant::now() {
            return Err(GatewayServiceLeaseError::ElapsedDeadline);
        }
        let cancellation = CancellationToken::new();
        let (status, _) = watch::channel(GatewayServiceLeaseStatus::Active {
            lease: lease.clone(),
            deadline: initial_deadline,
            last_retry: None,
        });
        let control = GatewayServiceLeaseControl {
            cancellation: cancellation.clone(),
            status: status.clone(),
        };
        Ok((
            Self {
                ownership,
                lease,
                owner,
                policy,
                deadline: initial_deadline,
                cancellation,
                status,
            },
            control,
        ))
    }

    /// Runs renewals until explicit stop or durable ownership loss.
    pub async fn run(mut self) -> GatewayServiceLeaseRunResult {
        let Some(mut next_renewal) = Instant::now().checked_add(self.policy.renewal_interval)
        else {
            return self.mark_lost(GatewayServiceLeaseLossReason::Invalid);
        };
        loop {
            if self.cancellation.is_cancelled() {
                mark_stopped(&self.status);
                return GatewayServiceLeaseRunResult::Stopped;
            }
            if Instant::now() >= self.deadline {
                return self.mark_lost(GatewayServiceLeaseLossReason::Expired);
            }
            let wake_at = next_renewal.min(self.deadline);
            tokio::select! {
                () = self.cancellation.cancelled() => {
                    mark_stopped(&self.status);
                    return GatewayServiceLeaseRunResult::Stopped;
                }
                () = time::sleep_until(wake_at) => {}
            }
            if Instant::now() >= self.deadline {
                return self.mark_lost(GatewayServiceLeaseLossReason::Expired);
            }

            let call_started = Instant::now();
            let current_lease = self.lease.clone();
            let renewal = {
                let renewal_future =
                    self.ownership
                        .renew(&current_lease, &self.owner, self.policy.lease_duration);
                tokio::pin!(renewal_future);
                tokio::select! {
                    () = self.cancellation.cancelled() => {
                        mark_stopped(&self.status);
                        return GatewayServiceLeaseRunResult::Stopped;
                    }
                    result = time::timeout_at(self.deadline, &mut renewal_future) => result,
                }
            };
            match renewal {
                Err(_) => return self.mark_lost(GatewayServiceLeaseLossReason::Expired),
                Ok(Ok(updated)) => {
                    if Instant::now() >= self.deadline {
                        return self.mark_lost(GatewayServiceLeaseLossReason::Expired);
                    }
                    if let Err(reason) = validate_lease(&updated, &current_lease, &self.owner) {
                        return self.mark_lost(reason);
                    }
                    let Some(database_budget) = database_budget(&updated) else {
                        return self.mark_lost(GatewayServiceLeaseLossReason::Invalid);
                    };
                    let Some(call_deadline) = call_started.checked_add(self.policy.lease_duration)
                    else {
                        return self.mark_lost(GatewayServiceLeaseLossReason::Invalid);
                    };
                    let Some(database_deadline) = call_started.checked_add(database_budget) else {
                        return self.mark_lost(GatewayServiceLeaseLossReason::Invalid);
                    };
                    let deadline = call_deadline.min(database_deadline);
                    if deadline <= Instant::now() {
                        return self.mark_lost(GatewayServiceLeaseLossReason::Expired);
                    }
                    self.lease = updated;
                    self.deadline = deadline;
                    self.publish_active(None);
                    let Some(next) = call_started.checked_add(self.policy.renewal_interval) else {
                        return self.mark_lost(GatewayServiceLeaseLossReason::Invalid);
                    };
                    next_renewal = next;
                }
                Ok(Err(GatewayServiceOwnershipError::Unavailable)) => {
                    if Instant::now() >= self.deadline {
                        return self.mark_lost(GatewayServiceLeaseLossReason::Expired);
                    }
                    self.publish_active(Some(GatewayServiceLeaseRetryReason::Unavailable));
                    let Some(next) = Instant::now().checked_add(self.policy.renewal_interval)
                    else {
                        return self.mark_lost(GatewayServiceLeaseLossReason::Invalid);
                    };
                    next_renewal = next;
                }
                Ok(Err(error)) => return self.mark_lost(loss_reason(error)),
            }
        }
    }

    fn publish_active(&self, last_retry: Option<GatewayServiceLeaseRetryReason>) {
        let lease = self.lease.clone();
        let deadline = self.deadline;
        self.status.send_modify(|status| {
            if matches!(status, GatewayServiceLeaseStatus::Active { .. }) {
                *status = GatewayServiceLeaseStatus::Active {
                    lease,
                    deadline,
                    last_retry,
                };
            }
        });
    }

    fn mark_lost(&self, reason: GatewayServiceLeaseLossReason) -> GatewayServiceLeaseRunResult {
        self.cancellation.cancel();
        self.status
            .send_replace(GatewayServiceLeaseStatus::Lost(reason));
        GatewayServiceLeaseRunResult::Lost(reason)
    }
}

impl<O: ?Sized> Drop for GatewayServiceLeaseMonitor<O> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        mark_stopped(&self.status);
    }
}

fn mark_stopped(status: &watch::Sender<GatewayServiceLeaseStatus>) {
    status.send_modify(|current| {
        if matches!(current, GatewayServiceLeaseStatus::Active { .. }) {
            *current = GatewayServiceLeaseStatus::Stopped;
        }
    });
}

fn validate_lease(
    lease: &GatewayServiceInstanceLease,
    expected: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> Result<(), GatewayServiceLeaseLossReason> {
    if lease.identity != expected.identity || lease.vm_id != expected.vm_id {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.owner_host_id != owner.host_id {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.owner_uuid != owner.owner_uuid {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.fencing_token != expected.fencing_token || lease.fencing_token <= 0 {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.vm_id != format!("gateway-service-{}", lease.identity.instance_id) {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.lease_expires_at <= lease.heartbeat_at {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if !lease.state.is_live()
        || lease.identity.instance_id.is_nil()
        || lease.identity.gateway_id.is_nil()
        || lease.identity.revision_id.is_nil()
    {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    Ok(())
}

fn database_budget(lease: &GatewayServiceInstanceLease) -> Option<Duration> {
    let budget = lease.lease_expires_at - lease.heartbeat_at;
    Duration::try_from(budget)
        .ok()
        .filter(|value| !value.is_zero())
}

const fn loss_reason(error: GatewayServiceOwnershipError) -> GatewayServiceLeaseLossReason {
    match error {
        GatewayServiceOwnershipError::StaleLease => GatewayServiceLeaseLossReason::Stale,
        GatewayServiceOwnershipError::Conflict => GatewayServiceLeaseLossReason::Conflict,
        GatewayServiceOwnershipError::InvalidArgument
        | GatewayServiceOwnershipError::Unavailable => GatewayServiceLeaseLossReason::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayServiceInstanceState;
    use async_trait::async_trait;
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tokio::{sync::Notify, time::Duration};
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

    #[test]
    fn rejects_invalid_policy() {
        assert!(
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(1),
                renewal_interval: Duration::from_secs(1),
            }
            .validate()
            .is_err()
        );
        let owner = owner();
        let mut malformed = lease(&owner, GatewayServiceInstanceState::Starting);
        malformed.vm_id = "wrong-vm".to_owned();
        malformed.lease_expires_at = malformed.heartbeat_at;
        let result = GatewayServiceLeaseMonitor::new(
            MockOwnership::new(Vec::new()),
            malformed,
            owner,
            policy(),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(matches!(
            result,
            Err(GatewayServiceLeaseError::InvalidIdentity)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn renewal_latency_cannot_extend_prior_deadline() {
        let owner = owner();
        let initial = lease(&owner, GatewayServiceInstanceState::Starting);
        let ownership = MockOwnership::new(vec![RenewAction::Success {
            lease: initial.clone(),
            delay: Duration::from_millis(100),
        }]);
        let started = ownership.started.clone();
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            ownership.clone(),
            initial,
            owner,
            policy(),
            Instant::now() + Duration::from_millis(50),
        )
        .expect("monitor");
        let status = control.subscribe();
        let task = tokio::spawn(monitor.run());
        tokio::task::yield_now().await;
        let entered = started.notified();
        tokio::time::advance(Duration::from_millis(20)).await;
        entered.await;
        assert_eq!(ownership.calls.load(Ordering::Relaxed), 1);
        tokio::time::advance(Duration::from_millis(30)).await;
        assert_eq!(
            task.await.expect("monitor task"),
            GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Expired)
        );
        assert_eq!(
            *status.borrow(),
            GatewayServiceLeaseStatus::Lost(GatewayServiceLeaseLossReason::Expired)
        );
        drop(control);
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_retries_only_until_expiry() {
        let owner = owner();
        let initial = lease(&owner, GatewayServiceInstanceState::Starting);
        let ownership = MockOwnership::new(vec![RenewAction::Failure {
            error: GatewayServiceOwnershipError::Unavailable,
            delay: Duration::ZERO,
        }]);
        let started = ownership.started.clone();
        let initial_deadline = Instant::now() + Duration::from_millis(50);
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            ownership.clone(),
            initial,
            owner,
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_millis(80),
                renewal_interval: Duration::from_millis(10),
            },
            initial_deadline,
        )
        .expect("monitor");
        let status = control.subscribe();
        let task = tokio::spawn(monitor.run());
        tokio::task::yield_now().await;
        let entered = started.notified();
        tokio::time::advance(Duration::from_millis(10)).await;
        entered.await;
        tokio::task::yield_now().await;
        assert_eq!(ownership.calls.load(Ordering::Relaxed), 1);
        assert!(
            matches!(&*status.borrow(), GatewayServiceLeaseStatus::Active {
            deadline,
            last_retry: Some(GatewayServiceLeaseRetryReason::Unavailable),
            ..
        } if *deadline == initial_deadline)
        );
        let retry_entered = started.notified();
        tokio::time::advance(Duration::from_millis(10)).await;
        retry_entered.await;
        assert_eq!(ownership.calls.load(Ordering::Relaxed), 2);
        tokio::time::advance(Duration::from_millis(30)).await;
        assert_eq!(
            task.await.expect("monitor task"),
            GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Expired)
        );
        assert_eq!(
            *status.borrow(),
            GatewayServiceLeaseStatus::Lost(GatewayServiceLeaseLossReason::Expired)
        );
        drop(control);
    }

    #[tokio::test(start_paused = true)]
    async fn stale_and_malformed_renewals_are_terminal() {
        let owner2 = owner();
        let initial = lease(&owner2, GatewayServiceInstanceState::Starting);
        let mut malformed = initial.clone();
        malformed.fencing_token += 1;
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            MockOwnership::new(vec![RenewAction::Success {
                lease: malformed,
                delay: Duration::ZERO,
            }]),
            initial.clone(),
            owner2.clone(),
            policy(),
            Instant::now() + Duration::from_millis(80),
        )
        .expect("monitor");
        let task = tokio::spawn(monitor.run());
        tokio::time::advance(Duration::from_millis(20)).await;
        assert_eq!(
            task.await.expect("monitor task"),
            GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Invalid)
        );
        drop(control);

        let mut changed_identity = initial.clone();
        changed_identity.identity.instance_id = Uuid::new_v4();
        assert_invalid_renewal(owner2.clone(), initial.clone(), changed_identity).await;
        let mut changed_owner = initial.clone();
        changed_owner.owner_host_id = "other-host".to_owned();
        assert_invalid_renewal(owner2.clone(), initial.clone(), changed_owner).await;
        let mut changed_daemon = initial.clone();
        changed_daemon.owner_uuid = Uuid::new_v4();
        assert_invalid_renewal(owner2.clone(), initial.clone(), changed_daemon).await;
        let mut changed_vm = initial.clone();
        changed_vm.vm_id = "gateway-service-other".to_owned();
        assert_invalid_renewal(owner2.clone(), initial.clone(), changed_vm).await;

        let owner = owner();
        let initial = lease(&owner, GatewayServiceInstanceState::Starting);
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            MockOwnership::new(vec![RenewAction::Failure {
                error: GatewayServiceOwnershipError::StaleLease,
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
        assert_eq!(
            task.await.expect("monitor task"),
            GatewayServiceLeaseRunResult::Lost(GatewayServiceLeaseLossReason::Stale)
        );
        drop(control);
    }

    #[tokio::test(start_paused = true)]
    async fn successful_renewal_preserves_lifecycle_state_changes() {
        let owner = owner();
        let initial = lease(&owner, GatewayServiceInstanceState::Starting);
        let mut ready = initial.clone();
        ready.state = GatewayServiceInstanceState::Ready;
        ready.lease_expires_at = ready.heartbeat_at + ::time::Duration::milliseconds(40);
        let mut draining = ready.clone();
        draining.state = GatewayServiceInstanceState::Draining;
        draining.lease_expires_at = draining.heartbeat_at + ::time::Duration::seconds(10);
        let ownership = MockOwnership::new(vec![
            RenewAction::Success {
                lease: ready,
                delay: Duration::from_millis(30),
            },
            RenewAction::Success {
                lease: draining,
                delay: Duration::from_millis(5),
            },
        ]);
        let started = ownership.started.clone();
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            ownership.clone(),
            initial,
            owner,
            policy(),
            Instant::now() + Duration::from_millis(200),
        )
        .expect("monitor");
        let status = control.subscribe();
        let task = tokio::spawn(monitor.run());
        tokio::task::yield_now().await;
        let entered = started.notified();
        tokio::time::advance(Duration::from_millis(20)).await;
        entered.await;
        let second_entered = started.notified();
        tokio::time::advance(Duration::from_millis(30)).await;
        tokio::task::yield_now().await;
        assert!(
            matches!(&*status.borrow(), GatewayServiceLeaseStatus::Active { lease, deadline, .. } if lease.state == GatewayServiceInstanceState::Ready
                && *deadline == ownership.call_starts.lock().expect("call start mutex")[0] + Duration::from_millis(40))
        );
        second_entered.await;
        tokio::time::advance(Duration::from_millis(5)).await;
        tokio::task::yield_now().await;
        assert!(
            matches!(&*status.borrow(), GatewayServiceLeaseStatus::Active { lease, deadline, .. } if lease.state == GatewayServiceInstanceState::Draining
                && *deadline == ownership.call_starts.lock().expect("call start mutex")[1] + policy().lease_duration)
        );
        control.stop();
        assert_eq!(
            task.await.expect("monitor task"),
            GatewayServiceLeaseRunResult::Stopped
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_or_dropping_control_marks_status_unavailable() {
        let owner = owner();
        let initial = lease(&owner, GatewayServiceInstanceState::Starting);
        let ownership = MockOwnership::new(vec![RenewAction::Success {
            lease: initial.clone(),
            delay: Duration::from_secs(1),
        }]);
        let started = ownership.started.clone();
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            ownership,
            initial,
            owner,
            policy(),
            Instant::now() + Duration::from_secs(1),
        )
        .expect("monitor");
        let status = control.subscribe();
        let task = tokio::spawn(monitor.run());
        tokio::task::yield_now().await;
        let entered = started.notified();
        tokio::time::advance(Duration::from_millis(20)).await;
        entered.await;
        drop(control);
        assert_eq!(*status.borrow(), GatewayServiceLeaseStatus::Stopped);
        assert_eq!(
            task.await.expect("monitor task"),
            GatewayServiceLeaseRunResult::Stopped
        );
    }
}

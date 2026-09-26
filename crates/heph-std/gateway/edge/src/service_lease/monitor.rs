use std::sync::Arc;
use tokio::{
    sync::watch,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;

use super::{
    types::{
        GatewayServiceLeaseControl, GatewayServiceLeaseError, GatewayServiceLeaseLossReason,
        GatewayServiceLeasePolicy, GatewayServiceLeaseRetryReason, GatewayServiceLeaseRunResult,
        GatewayServiceLeaseStatus,
    },
    validation::{database_budget, loss_reason, mark_stopped, validate_lease},
};
use crate::{
    GatewayServiceInstanceLease, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError,
};

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

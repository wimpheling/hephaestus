use std::time::Duration;
use tokio::{sync::watch, time::Instant};
use tokio_util::sync::CancellationToken;

use super::validation::mark_stopped;
use crate::{GatewayServiceInstanceLease, MAX_SERVICE_OWNERSHIP_LEASE};

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
    pub(super) cancellation: CancellationToken,
    pub(super) status: watch::Sender<GatewayServiceLeaseStatus>,
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

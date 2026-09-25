//! Fenced orchestration of physical and durable service cleanup.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio::{
    sync::watch,
    time::{self, Instant},
};
use vm_trait::VmProvider;

use crate::{
    GatewayServiceCleanup, GatewayServiceFailure, GatewayServiceFailureCode,
    GatewayServiceFailureStore, GatewayServiceFailureStoreError, GatewayServiceIdentity,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceLaunchResolver,
    GatewayServiceLeaseControl, GatewayServiceLeaseLossReason, GatewayServiceLeaseMonitor,
    GatewayServiceLeasePolicy, GatewayServiceLeaseRunResult, GatewayServiceLeaseStatus,
    GatewayServiceOwner, GatewayServiceOwnership, GatewayServiceOwnershipError,
    GatewayServiceTargetStore,
};

type MonitorFuture = Pin<Box<dyn Future<Output = GatewayServiceLeaseRunResult> + Send>>;

/// Maximum time spent on one read-only confirmation after a lease is lost.
pub const MAX_SERVICE_CLEANUP_DATABASE_WAIT: Duration = Duration::from_secs(30);

/// Policy for fenced cleanup orchestration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceCleanupDriverPolicy {
    /// Lease renewal and local deadline policy.
    pub lease: GatewayServiceLeasePolicy,
    /// Bound for one durable cleanup or confirmation operation.
    pub database_timeout: Duration,
}

impl GatewayServiceCleanupDriverPolicy {
    /// Validates cleanup timing bounds.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupDriverError::InvalidInput`] when either
    /// the lease or database timeout policy is outside its bounds.
    pub fn validate(self) -> Result<(), GatewayServiceCleanupDriverError> {
        self.lease
            .validate()
            .map_err(|_| GatewayServiceCleanupDriverError::InvalidInput)?;
        if self.database_timeout.is_zero()
            || self.database_timeout > MAX_SERVICE_CLEANUP_DATABASE_WAIT
        {
            return Err(GatewayServiceCleanupDriverError::InvalidInput);
        }
        Ok(())
    }
}

/// Redacted errors from the durable cleanup boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceCleanupDriverError {
    /// The driver, lease, or policy was malformed.
    #[error("invalid gateway service cleanup driver input")]
    InvalidInput,
    /// The exact owner or fence is no longer current; no stale write occurred.
    #[error("gateway service cleanup owner is stale")]
    Stale,
    /// Durable storage was unavailable; all caller-owned progress is retained.
    #[error("gateway service cleanup durable storage is unavailable")]
    Unavailable,
    /// The physical or durable operation was attempted after malformed input.
    #[error("gateway service cleanup durable operation was rejected")]
    Rejected,
}

/// Result of one cleanup attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceCleanupDriverOutcome {
    /// Physical and durable cleanup are confirmed.
    Cleaned,
    /// The caller retains state and must retry later.
    Pending {
        /// Whether provider teardown and materializer cleanup are complete.
        physical_complete: bool,
        /// Whether the pending failure has been durably recorded.
        failure_recorded: bool,
        /// Whether lease monitoring lost the current claim during this attempt.
        lease_lost: bool,
    },
}

/// Caller-owned fenced cleanup driver.
pub struct GatewayServiceCleanupDriver {
    ownership: Arc<dyn GatewayServiceOwnership>,
    failure_store: Arc<dyn GatewayServiceFailureStore>,
    targets: Arc<dyn GatewayServiceTargetStore>,
    provider: Arc<dyn VmProvider>,
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    owner: GatewayServiceOwner,
    policy: GatewayServiceCleanupDriverPolicy,
}

impl GatewayServiceCleanupDriver {
    /// Creates a driver with immutable dependency and owner snapshots.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupDriverError::InvalidInput`] for an
    /// invalid owner or policy.
    pub fn new(
        ownership: Arc<dyn GatewayServiceOwnership>,
        failure_store: Arc<dyn GatewayServiceFailureStore>,
        targets: Arc<dyn GatewayServiceTargetStore>,
        provider: Arc<dyn VmProvider>,
        resolver: Arc<dyn GatewayServiceLaunchResolver>,
        owner: GatewayServiceOwner,
        policy: GatewayServiceCleanupDriverPolicy,
    ) -> Result<Self, GatewayServiceCleanupDriverError> {
        owner
            .validate()
            .map_err(|_| GatewayServiceCleanupDriverError::InvalidInput)?;
        policy.validate()?;
        Ok(Self {
            ownership,
            failure_store,
            targets,
            provider,
            resolver,
            owner,
            policy,
        })
    }
}

#[path = "service_cleanup_driver/attempt.rs"]
mod attempt;
#[path = "service_cleanup_driver/confirmation.rs"]
mod confirmation;
#[path = "service_cleanup_driver/monitor.rs"]
mod monitor;
#[path = "service_cleanup_driver/renewal.rs"]
mod renewal;

#[cfg(test)]
#[path = "service_cleanup_driver/tests.rs"]
mod tests;

//! Parent-owned boot recovery for abandoned persistent service instances.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio::time::Instant;
use vm_trait::VmProvider;

use crate::{
    GatewayServiceCleanup, GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError,
    GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverPolicy,
    GatewayServiceExpiredClaimRecovery, GatewayServiceFailure, GatewayServiceFailureStore,
    GatewayServiceInstanceLease, GatewayServiceInstancePage, GatewayServiceInstanceState,
    GatewayServiceLaunchResolver, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError, GatewayServiceTargetStore, MAX_SERVICE_INSTANCE_PAGE_SIZE,
};

/// Maximum number of cleanup attempts owned concurrently during boot recovery.
pub const MAX_SERVICE_BOOT_RECOVERY_CLEANUPS: usize = 2;
pub const BOOT_RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Immutable dependencies for the boot recovery gate.
pub struct GatewayServiceBootRecoveryContext {
    /// Stable host and daemon incarnation performing recovery.
    pub owner: GatewayServiceOwner,
    /// Lease and durable-operation bounds used by cleanup.
    pub cleanup_policy: GatewayServiceCleanupDriverPolicy,
    /// Physical/materializer cleanup timeout.
    pub shutdown_timeout: Duration,
    /// Durable ownership adapter for bounded batch discovery.
    pub ownership: Arc<dyn GatewayServiceOwnership>,
    /// Exact-instance takeover adapter for retained retries.
    pub exact_recovery: Arc<dyn GatewayServiceExpiredClaimRecovery>,
    /// Host inventory and exact-instance lookup adapter.
    pub targets: Arc<dyn GatewayServiceTargetStore>,
    /// Redacted durable failure store.
    pub failure_store: Arc<dyn GatewayServiceFailureStore>,
    /// Exact launch/materializer resolver.
    pub resolver: Arc<dyn GatewayServiceLaunchResolver>,
    /// VM provider used for orphan cleanup.
    pub provider: Arc<dyn VmProvider>,
}

/// Redacted boot-gate errors. Any error leaves the gate closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceBootRecoveryError {
    /// The immutable context or a durable lease was malformed.
    #[error("invalid gateway service boot recovery input")]
    InvalidInput,
    /// Durable recovery was unavailable; the caller may poll again.
    #[error("gateway service boot recovery is unavailable")]
    Unavailable,
}

/// One parent-visible boot recovery step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceBootRecoveryEvent {
    /// Recovery is waiting for an expired lease or a bounded retry delay.
    Waiting,
    /// One cleanup job finished but retained state for a later retry.
    Pending,
    /// No non-cleaned host rows remain after a fresh first-page proof.
    Complete,
}

/// Exact unresolved ownership returned when boot recovery is consumed.
pub struct GatewayServiceBootRecoveryUnresolved {
    /// Durable instance lease retained for later exact takeover.
    pub lease: GatewayServiceInstanceLease,
    /// Physical/materializer progress retained across attempts.
    pub cleanup: GatewayServiceCleanup,
    /// Failure report still awaiting durable recording.
    pub pending_failure: Option<GatewayServiceFailure>,
}

/// Result of consuming the boot gate after all owned futures settle.
pub struct GatewayServiceBootRecoveryShutdown {
    /// Claims and physical cleanup still requiring a later recovery pass.
    pub unresolved: Vec<GatewayServiceBootRecoveryUnresolved>,
    /// Raw claims retained when an adapter returned malformed ownership data.
    ///
    /// These rows are intentionally not converted into cleanup state: doing so
    /// would either invent a VM identity or discard durable ownership.
    pub unresolved_claims: Vec<GatewayServiceInstanceLease>,
}

pub struct BootCleanupState {
    pub lease: GatewayServiceInstanceLease,
    pub cleanup: GatewayServiceCleanup,
    pub pending_failure: Option<GatewayServiceFailure>,
}

pub struct ClaimCompletion {
    pub result: Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>,
    pub deadline: Instant,
    pub known_leases: Vec<GatewayServiceInstanceLease>,
}

pub struct CleanupCompletion {
    pub state: BootCleanupState,
    pub result: Result<GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverError>,
}

pub type ClaimFuture = Pin<Box<dyn Future<Output = ClaimCompletion> + Send>>;
pub type CleanupFuture = Pin<Box<dyn Future<Output = CleanupCompletion> + Send>>;

/// Parent-owned boot recovery gate.
pub struct GatewayServiceBootRecovery {
    context: Arc<GatewayServiceBootRecoveryContext>,
    pub(super) claim_future: Option<ClaimFuture>,
    pub(super) cleanup_jobs: Vec<CleanupFuture>,
    retained: Vec<BootCleanupState>,
    unresolved_claims: Vec<GatewayServiceInstanceLease>,
    blocked_on_malformed_claim: bool,
    next_retry_at: Option<Instant>,
    complete: bool,
}

#[path = "service_boot_recovery/gate.rs"]
mod gate;
#[path = "service_boot_recovery/helpers.rs"]
mod helpers;

#[cfg(test)]
#[path = "service_boot_recovery_tests/tests.rs"]
mod tests;

//! Parent-owned concurrent startup bookkeeping for persistent services.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Poll,
};

use tokio::{
    sync::watch,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vm_trait::VmProvider;

use crate::{
    GatewayServiceCapacity, GatewayServiceCapacityError, GatewayServiceCapacityToken,
    GatewayServiceClaimResolutionStore, GatewayServiceCleanup, GatewayServiceCleanupDriver,
    GatewayServiceCleanupDriverError, GatewayServiceCleanupDriverOutcome,
    GatewayServiceCleanupDriverPolicy, GatewayServiceCoordinator, GatewayServiceCoordinatorFailure,
    GatewayServiceCoordinatorFailureReason, GatewayServiceCoordinatorStatus,
    GatewayServiceExpiredClaimRecovery, GatewayServiceFailure, GatewayServiceFailureStore,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceLaunchResolver,
    GatewayServiceLogWriterConfig, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError, GatewayServiceRegistry, GatewayServiceStartupIntent,
    GatewayServiceSupervisorPolicy, GatewayServiceTargetStore,
};

/// Validated immutable dependencies shared by all startup jobs.
pub struct GatewayServiceSupervisorContext {
    /// Daemon owner identity used for every claim and coordinator.
    pub owner: GatewayServiceOwner,
    /// Bounded service lifecycle policy.
    pub policy: GatewayServiceSupervisorPolicy,
    /// Durable ownership adapter.
    pub ownership: Arc<dyn GatewayServiceOwnership>,
    /// Durable redacted failure adapter.
    pub failure_store: Arc<dyn GatewayServiceFailureStore>,
    /// Immutable service launch resolver.
    pub resolver: Arc<dyn GatewayServiceLaunchResolver>,
    /// VM provider used by coordinators.
    pub provider: Arc<dyn VmProvider>,
    /// Exact target lookup adapter used by coordinators.
    pub targets: Arc<dyn GatewayServiceTargetStore>,
    /// Shared ready-instance registry.
    pub registry: GatewayServiceRegistry,
    /// Host-owned authority used for readiness probes.
    pub service_authority: String,
}

/// A startup request selected by an already-running reconciliation caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceStartupRequest {
    /// Gateway whose service revision is being started.
    pub gateway_id: Uuid,
    /// Exact immutable service revision.
    pub revision_id: Uuid,
    /// Whether readiness should activate or restore this revision.
    pub intent: GatewayServiceStartupIntent,
}

/// Public lifecycle state for one parent-owned startup job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceSupervisorJobStatus {
    /// Capacity is reserved and the durable claim is in flight.
    Claiming,
    /// A claim exists and coordinator preparation is in flight.
    Starting,
    /// The coordinator reached readiness and startup capacity was released.
    Ready,
    /// The job ended with all physical and durable cleanup complete.
    Settled,
    /// The job ended while retaining an exact claim or cleanup responsibility.
    Failed,
    /// The claim operation may have committed but did not return a result.
    Uncertain,
    /// The caller requested cancellation and the job retained cleanup state.
    Cancelled,
    /// A parent-owned cleanup retry is currently running.
    CleanupPending,
}

/// A caller-owned handle for cancellation and status observation.
#[derive(Debug)]
pub struct GatewayServiceStartupHandle {
    id: Uuid,
    cancellation: CancellationToken,
    drain: watch::Sender<bool>,
    status: watch::Receiver<GatewayServiceSupervisorJobStatus>,
}

impl GatewayServiceStartupHandle {
    /// Returns the stable job identifier.
    #[must_use]
    pub const fn job_id(&self) -> Uuid {
        self.id
    }

    /// Requests cancellation while preserving the supervisor-owned job.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Requests graceful retirement of the service after accepted calls drain.
    pub fn request_drain(&self) {
        self.drain.send_replace(true);
    }

    /// Subscribes to lifecycle status changes.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<GatewayServiceSupervisorJobStatus> {
        self.status.clone()
    }
}

/// Completion notification returned by [`GatewayServiceSupervisor::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceSupervisorEvent {
    /// Completed job identifier.
    pub job_id: Uuid,
    /// Terminal public status.
    pub status: GatewayServiceSupervisorJobStatus,
    /// Whether the capacity reservation has been released.
    pub capacity_released: bool,
}

/// Exact unresolved state returned when the supervisor is consumed at shutdown.
pub struct GatewayServiceSupervisorUnresolved {
    /// Job that still owns this state.
    pub job_id: Uuid,
    /// Exact gateway/revision request needed by later reconciliation.
    pub request: GatewayServiceStartupRequest,
    /// Exact capacity reservation retained for later reconciliation.
    pub capacity_token: GatewayServiceCapacityToken,
    /// Late durable claim, when one was returned.
    pub lease: Option<GatewayServiceInstanceLease>,
    /// Whether the claim operation ended without an authoritative result.
    pub claim_uncertain: bool,
    /// Coordinator failure retaining any VM or materialization responsibility.
    pub coordinator_failure: Option<GatewayServiceCoordinatorFailure>,
    /// Physical and materializer cleanup progress retained for retry.
    pub cleanup: Option<GatewayServiceCleanup>,
    /// Failure report still awaiting durable recording.
    pub pending_failure: Option<GatewayServiceFailure>,
    /// Original coordinator termination reason.
    pub original_reason: Option<GatewayServiceCoordinatorFailureReason>,
}

/// Result of consuming a supervisor after cancellation and job settlement.
pub struct GatewayServiceSupervisorShutdown {
    /// Jobs whose capacity and cleanup responsibility remain unresolved.
    pub unresolved: Vec<GatewayServiceSupervisorUnresolved>,
}

/// Construction and start failures which do not transfer ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceSupervisorError {
    /// Context, request, or deadline input was malformed.
    #[error("invalid gateway service supervisor input")]
    InvalidInput,
    /// Capacity was unavailable before any durable claim was attempted.
    #[error("gateway service startup capacity is unavailable")]
    Capacity(#[from] GatewayServiceCapacityError),
    /// A job for this exact gateway and revision already exists.
    #[error("gateway service startup is already reserved")]
    Duplicate,
    /// No terminal coordinator cleanup exists for this job.
    #[error("gateway service cleanup retry is not eligible")]
    RetryNotEligible,
    /// The requested cleanup job no longer exists.
    #[error("gateway service cleanup retry job was not found")]
    RetryNotFound,
    /// A cleanup retry is already parent-owned and running.
    #[error("gateway service cleanup retry is already in flight")]
    RetryAlreadyInFlight,
    /// The claim-resolution store could not complete its bounded read.
    #[error("gateway service claim resolution is unavailable")]
    ClaimResolutionUnavailable,
}

/// Parent-owned bounded set of concurrent startup jobs.
pub struct GatewayServiceSupervisor {
    context: Arc<GatewayServiceSupervisorContext>,
    log_writer: Option<GatewayServiceLogWriterConfig>,
    capacity: Arc<Mutex<GatewayServiceCapacity>>,
    jobs: Vec<Pin<Box<dyn Future<Output = JobCompletion> + Send>>>,
    cleanup_jobs: Vec<Pin<Box<dyn Future<Output = CleanupCompletion> + Send>>>,
    claim_resolution_jobs: Vec<Pin<Box<dyn Future<Output = ClaimResolutionCompletion> + Send>>>,
    records: HashMap<Uuid, JobRecord>,
}

#[derive(Clone, Copy)]
struct StartupDeadlines {
    initial_lease_deadline: Instant,
    startup_deadline: Instant,
}

#[path = "service_supervisor/claim.rs"]
mod claim;
#[path = "service_supervisor/claim_resolution.rs"]
mod claim_resolution;
#[path = "service_supervisor/cleanup.rs"]
mod cleanup;
#[path = "service_supervisor/cleanup_driver.rs"]
mod cleanup_driver;
#[path = "service_supervisor/job.rs"]
mod job;
#[path = "service_supervisor/poll.rs"]
mod poll;
#[path = "service_supervisor/poll_futures.rs"]
mod poll_futures;
#[path = "service_supervisor/shutdown.rs"]
mod shutdown;
#[path = "service_supervisor/startup.rs"]
mod startup;

fn failure_from_retry(state: &CleanupRetryState) -> GatewayServiceCoordinatorFailure {
    GatewayServiceCoordinatorFailure {
        identity: state.cleanup.identity(),
        lease: state.lease.clone(),
        reason: state
            .reason
            .unwrap_or(GatewayServiceCoordinatorFailureReason::Ownership),
        vm: state.cleanup.retained_vm(),
        materialization_owned: !state.cleanup.materializer_cleanup_confirmed(),
        physical_cleanup_complete: state.cleanup.vm_teardown_confirmed()
            && state.cleanup.materializer_cleanup_confirmed(),
        durable_cleanup_complete: false,
        pending_failure: state.pending_failure,
    }
}

struct JobRecord {
    request: GatewayServiceStartupRequest,
    token: GatewayServiceCapacityToken,
    cancellation: CancellationToken,
    status: watch::Sender<GatewayServiceSupervisorJobStatus>,
    capacity_retained: bool,
    completion: Option<JobTerminal>,
    cleanup_retry: Option<CleanupRetryState>,
    cleanup_in_flight: bool,
    claim_resolution_in_flight: bool,
}

struct JobCompletion {
    id: Uuid,
    status: GatewayServiceSupervisorJobStatus,
    capacity_released: bool,
    terminal: JobTerminal,
}

struct JobTerminal {
    lease: Option<GatewayServiceInstanceLease>,
    claim_uncertain: bool,
    coordinator_failure: Option<GatewayServiceCoordinatorFailure>,
    claim_cleanup_reason: Option<GatewayServiceCoordinatorFailureReason>,
}

struct CleanupRetryState {
    cleanup: GatewayServiceCleanup,
    lease: GatewayServiceInstanceLease,
    pending_failure: Option<GatewayServiceFailure>,
    reason: Option<GatewayServiceCoordinatorFailureReason>,
}

struct CleanupCompletion {
    id: Uuid,
    state: CleanupRetryState,
    result: Result<(), GatewayServiceCleanupDriverError>,
}

enum ClaimResolutionResult {
    Absent,
    Owned(CleanupRetryState),
    Retained(Option<GatewayServiceInstanceLease>),
}

struct ClaimResolutionCompletion {
    id: Uuid,
    result: Result<ClaimResolutionResult, GatewayServiceSupervisorError>,
}

enum SupervisorCompletion {
    Startup(JobCompletion),
    Cleanup(CleanupCompletion),
    ClaimResolution(ClaimResolutionCompletion),
}
#[cfg(test)]
#[path = "service_supervisor/tests.rs"]
mod tests;

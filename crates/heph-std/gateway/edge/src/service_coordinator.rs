//! Parent-owned startup, readiness, and cleanup for one service instance.

use std::{
    fmt,
    future::{self, Future},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use tokio::{
    sync::watch,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;
use vm_trait::{VmInstance, VmProvider};

use crate::{
    DEFAULT_SERVICE_LOG_FINAL_FLUSH_TIMEOUT, GatewayEdgeError, GatewayServiceFailure,
    GatewayServiceFailureCode, GatewayServiceFailureStore, GatewayServiceFailureStoreError,
    GatewayServiceIdentity, GatewayServiceInstanceKey, GatewayServiceInstanceLease,
    GatewayServiceInstanceState, GatewayServiceLaunchRequest, GatewayServiceLaunchResolver,
    GatewayServiceLeaseControl, GatewayServiceLeaseMonitor, GatewayServiceLeaseRunResult,
    GatewayServiceLeaseStatus, GatewayServiceLogStore, GatewayServiceOwner,
    GatewayServiceOwnership, GatewayServiceOwnershipError, GatewayServiceRegistry,
    GatewayServiceSupervisorPolicy, GatewayServiceTargetStore, PreparedGatewayService,
    ServiceInstanceError, ServiceInstanceHandle, ServiceInstancePolicy, ServiceLogWriter,
    ServiceLogWriterPolicy, ServiceLogWriterPoll, ServicePreparationFailure, ServiceWorkerState,
    new_service_instance, new_service_preparation,
};

type MonitorFuture = Pin<Box<dyn Future<Output = GatewayServiceLeaseRunResult> + Send>>;
type WorkerFuture = Pin<Box<dyn Future<Output = Result<(), ServiceInstanceError>> + Send>>;
type LogWriter = ServiceLogWriter<dyn GatewayServiceLogStore>;

#[path = "service_coordinator/cleanup_lifecycle.rs"]
mod cleanup_lifecycle;
#[path = "service_coordinator/construction.rs"]
mod construction;
#[path = "service_coordinator/drain_lifecycle.rs"]
mod drain_lifecycle;
#[path = "service_coordinator/monitored_waits.rs"]
mod monitored_waits;
#[path = "service_coordinator/runtime.rs"]
mod runtime;
#[path = "service_coordinator/runtime_helpers.rs"]
mod runtime_helpers;
#[cfg(test)]
pub use runtime_helpers::drive_worker_with_logs;
use runtime_helpers::{cleanup_report, failure_for_reason};
#[path = "service_coordinator/serving.rs"]
mod serving;
#[path = "service_coordinator/transition_helpers.rs"]
mod transition_helpers;

/// Optional parent-owned durable application-log writer configuration.
#[derive(Clone)]
pub struct GatewayServiceLogWriterConfig {
    /// Worker-authorized durable append port.
    pub store: Arc<dyn GatewayServiceLogStore>,
    /// Bounded append and retry policy.
    pub policy: ServiceLogWriterPolicy,
}

impl GatewayServiceLogWriterConfig {
    /// Creates an optional writer configuration for application-log services.
    #[must_use]
    pub const fn new(
        store: Arc<dyn GatewayServiceLogStore>,
        policy: ServiceLogWriterPolicy,
    ) -> Self {
        Self { store, policy }
    }
}

/// Durable intent for one claimed service instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceStartupIntent {
    /// Start the desired candidate and promote it after readiness.
    ActivateDesired,
    /// Restore the exact active revision without changing revision pointers.
    RestoreActive,
}

/// Observable lifecycle state of one coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceCoordinatorStatus {
    /// Resolving, materializing, and provisioning.
    Preparing,
    /// Durable claim is moving into startup ownership.
    Starting,
    /// VM is starting or being probed for readiness.
    Probing,
    /// Exact fenced VM is registered and serving.
    Ready,
    /// Durable ownership is draining while accepted calls finish.
    Draining,
    /// Cleanup is stopping the worker and durable claim.
    Stopping,
    /// Physical and durable cleanup completed.
    Stopped,
    /// Cleanup or ownership responsibility remains for the parent.
    Failed,
}

/// Cancellation and status control for one coordinator.
pub struct GatewayServiceCoordinatorControl {
    cancellation: CancellationToken,
    drain: watch::Sender<bool>,
    status: watch::Sender<GatewayServiceCoordinatorStatus>,
}

impl GatewayServiceCoordinatorControl {
    /// Requests shutdown; the run future still settles owned work.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Coalesces a parent request to retire the ready service instance.
    pub fn request_drain(&self) {
        self.drain.send_replace(true);
    }

    /// Subscribes to coordinator lifecycle state.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<GatewayServiceCoordinatorStatus> {
        self.status.subscribe()
    }
}

impl Drop for GatewayServiceCoordinatorControl {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Redacted primary reason for coordinator termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceCoordinatorFailureReason {
    /// The parent requested shutdown.
    Cancelled,
    /// The claim was lost or its local lease deadline elapsed.
    LeaseLost,
    /// The startup/readiness deadline elapsed.
    StartupDeadline,
    /// Launch resolution or materialization failed.
    Preparation,
    /// A durable ownership transition failed.
    Ownership,
    /// The worker or registry contract failed.
    Runtime,
    /// Restore-active validation no longer matched the durable target.
    TargetUnavailable,
    /// Consecutive serving health probes exceeded the configured threshold.
    Health,
    /// Provider or materializer cleanup was not confirmed.
    CleanupIncomplete,
    /// All accepted invocations drained before retirement.
    Drained,
    /// The bounded drain grace period elapsed before retirement.
    DrainDeadline,
}

/// Failure retaining the latest lease and any live cleanup ownership.
pub struct GatewayServiceCoordinatorFailure {
    /// Exact service identity.
    pub identity: GatewayServiceIdentity,
    /// Most recent lease known by the coordinator.
    pub lease: GatewayServiceInstanceLease,
    /// Primary failure reason.
    pub reason: GatewayServiceCoordinatorFailureReason,
    /// VM handle retained when destruction did not complete.
    pub vm: Option<Arc<dyn VmInstance>>,
    /// Whether host materialization remains owned.
    pub materialization_owned: bool,
    /// Whether VM and materializer cleanup completed.
    pub physical_cleanup_complete: bool,
    /// Whether the durable row was confirmed `cleaned`.
    pub durable_cleanup_complete: bool,
    /// Failure report retained when durable recording did not complete.
    pub pending_failure: Option<GatewayServiceFailure>,
}

impl fmt::Debug for GatewayServiceCoordinatorFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GatewayServiceCoordinatorFailure")
            .field("identity", &self.identity)
            .field("lease", &self.lease)
            .field("reason", &self.reason)
            .field("vm_present", &self.vm.is_some())
            .field("materialization_owned", &self.materialization_owned)
            .field("physical_cleanup_complete", &self.physical_cleanup_complete)
            .field("durable_cleanup_complete", &self.durable_cleanup_complete)
            .field("pending_failure", &self.pending_failure)
            .finish()
    }
}

/// Construction errors for a service coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceCoordinatorError {
    /// The lease or owner identity is malformed.
    #[error("invalid gateway service coordinator identity")]
    InvalidIdentity,
    /// A deadline or lease policy is invalid.
    #[error("invalid gateway service coordinator lease policy")]
    InvalidLeasePolicy,
    /// The probe authority is not a valid HTTP authority.
    #[error("invalid gateway service coordinator probe authority")]
    InvalidAuthority,
}

/// Parent-owned coordinator for one exact claimed service instance.
pub struct GatewayServiceCoordinator {
    lease: GatewayServiceInstanceLease,
    owner: GatewayServiceOwner,
    intent: GatewayServiceStartupIntent,
    ownership: Arc<dyn GatewayServiceOwnership>,
    failure_store: Arc<dyn GatewayServiceFailureStore>,
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    provider: Arc<dyn VmProvider>,
    targets: Arc<dyn GatewayServiceTargetStore>,
    registry: GatewayServiceRegistry,
    service_authority: String,
    supervisor_policy: GatewayServiceSupervisorPolicy,
    startup_deadline: Instant,
    lease_control: GatewayServiceLeaseControl,
    lease_monitor: Option<GatewayServiceLeaseMonitor<dyn GatewayServiceOwnership>>,
    log_writer: Option<GatewayServiceLogWriterConfig>,
    cancellation: CancellationToken,
    drain: watch::Sender<bool>,
    drain_requested: watch::Receiver<bool>,
    status: watch::Sender<GatewayServiceCoordinatorStatus>,
}

type FailureReason = GatewayServiceCoordinatorFailureReason;

enum DrainTransition<T> {
    Started(T),
    Conflict,
    Deadline,
}

enum DrainCount {
    Zero,
    Active,
    Unavailable,
}

enum DrainOutcome {
    Conflict,
    Settled(Box<Result<(), GatewayServiceCoordinatorFailure>>),
}

enum PrepExit {
    Complete(Result<PreparedGatewayService, ServicePreparationFailure>),
    Cancelled(Result<PreparedGatewayService, ServicePreparationFailure>),
    StartupDeadline(Result<PreparedGatewayService, ServicePreparationFailure>),
    LeaseLost(Result<PreparedGatewayService, ServicePreparationFailure>),
}

impl PrepExit {
    const fn reason(&self) -> Option<FailureReason> {
        match self {
            Self::Complete(_) => None,
            Self::Cancelled(_) => Some(FailureReason::Cancelled),
            Self::StartupDeadline(_) => Some(FailureReason::StartupDeadline),
            Self::LeaseLost(_) => Some(FailureReason::LeaseLost),
        }
    }

    fn result(self) -> Result<PreparedGatewayService, ServicePreparationFailure> {
        match self {
            Self::Complete(result)
            | Self::Cancelled(result)
            | Self::StartupDeadline(result)
            | Self::LeaseLost(result) => result,
        }
    }
}

async fn monitor_signal(monitor: &mut MonitorFuture, done: bool) -> GatewayServiceLeaseRunResult {
    if done {
        future::pending().await
    } else {
        monitor.as_mut().await
    }
}

async fn await_with_monitor<T, F>(
    operation: &mut Pin<Box<F>>,
    monitor: &mut MonitorFuture,
    monitor_done: &mut bool,
    status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
) -> T
where
    F: Future<Output = T> + ?Sized,
{
    loop {
        tokio::select! {
            result = operation.as_mut() => return result,
            signal = monitor_signal(monitor, *monitor_done) => {
                let _ = signal;
                *monitor_done = true;
                return operation.as_mut().await;
            }
            changed = status.changed() => {
                if changed.is_err() || !lease_active(status) {
                    return operation.as_mut().await;
                }
            }
        }
    }
}

fn lease_active(status: &watch::Receiver<GatewayServiceLeaseStatus>) -> bool {
    matches!(&*status.borrow(), GatewayServiceLeaseStatus::Active { .. })
}
fn current_lease(
    status: &watch::Receiver<GatewayServiceLeaseStatus>,
) -> Option<GatewayServiceInstanceLease> {
    match &*status.borrow() {
        GatewayServiceLeaseStatus::Active { lease, .. } => Some(lease.clone()),
        GatewayServiceLeaseStatus::Lost(_) | GatewayServiceLeaseStatus::Stopped => None,
    }
}
fn current_deadline(status: &watch::Receiver<GatewayServiceLeaseStatus>) -> Option<Instant> {
    match &*status.borrow() {
        GatewayServiceLeaseStatus::Active { deadline, .. } => Some(*deadline),
        GatewayServiceLeaseStatus::Lost(_) | GatewayServiceLeaseStatus::Stopped => None,
    }
}

#[cfg(test)]
#[path = "service_coordinator/tests.rs"]
mod tests;

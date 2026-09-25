//! Lifecycle worker for one already-provisioned persistent gateway service VM.

use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use ::time::OffsetDateTime;
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;
use vm_trait::{StopMode, VmError, VmEvent, VmExit, VmInstance};

use crate::service_diagnostics::{ServiceDiagnostics, ServiceDiagnosticsSnapshot};
use crate::service_logs::ServiceLogBufferHandle;
use crate::{
    GatewayServiceFailure, GatewayServiceFailureCode, GatewayServiceLaunch,
    GatewayServiceLaunchResolver, ServiceProbeError, ServiceProbePolicy,
    probe_private_service_http,
};
use gateway_domain::ServiceLogCaptureMode;

const COMMAND_CAPACITY: usize = 8;
const MAX_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
/// Maximum duration for one VM stop, destroy, or cleanup operation.
pub const MAX_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// Observable state of the in-memory lifecycle worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceWorkerState {
    /// The worker owns a provisioned but not yet started VM.
    Provisioned,
    /// The VM is starting.
    Starting,
    /// The worker is probing the declared readiness endpoint.
    Probing,
    /// Readiness succeeded and bounded health requests are accepted.
    Ready,
    /// The VM is being stopped and destroyed.
    Stopping,
    /// Provider and materializer cleanup completed.
    Stopped,
    /// The instance failed before cleanup completed.
    Failed,
    /// Cleanup could not be completed; the materialized identity remains owned.
    CleanupIncomplete,
}

/// Bounded lifecycle policy selected by the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceInstancePolicy {
    /// Total time allowed for VM start and readiness.
    pub startup_timeout: Duration,
    /// Delay between failed readiness probes.
    pub probe_interval: Duration,
    /// Maximum time for one readiness or health probe.
    pub probe_timeout: Duration,
    /// Maximum time for each graceful stop and destroy operation.
    pub shutdown_timeout: Duration,
}

impl ServiceInstancePolicy {
    /// Creates the policy used by a service worker.
    #[must_use]
    pub const fn new(
        startup_timeout: Duration,
        probe_interval: Duration,
        probe_timeout: Duration,
        shutdown_timeout: Duration,
    ) -> Self {
        Self {
            startup_timeout,
            probe_interval,
            probe_timeout,
            shutdown_timeout,
        }
    }

    pub(crate) fn validate(self) -> Result<(), ServiceInstanceError> {
        if self.startup_timeout.is_zero()
            || self.startup_timeout > MAX_STARTUP_TIMEOUT
            || self.probe_interval.is_zero()
            || self.probe_interval > MAX_PROBE_INTERVAL
            || self.probe_timeout.is_zero()
            || self.probe_timeout > Duration::from_secs(30)
            || self.shutdown_timeout.is_zero()
            || self.shutdown_timeout > MAX_SHUTDOWN_TIMEOUT
        {
            return Err(ServiceInstanceError::InvalidPolicy);
        }
        Ok(())
    }
}

/// Redacted worker construction, lifecycle, and cleanup errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ServiceInstanceError {
    /// A policy, authority, launch identity, or service declaration is invalid.
    #[error("invalid gateway service instance configuration")]
    InvalidPolicy,
    /// Starting the known VM failed.
    #[error("gateway service VM startup failed")]
    StartupFailed,
    /// Readiness did not succeed before the startup deadline.
    #[error("gateway service readiness timed out")]
    StartupTimeout,
    /// The parent explicitly stopped the worker or dropped its last handle.
    #[error("gateway service shutdown requested")]
    Shutdown,
    /// The VM exited before or while serving.
    #[error("gateway service VM exited unexpectedly")]
    UnexpectedExit,
    /// A health request was made before readiness.
    #[error("gateway service is not ready")]
    NotReady,
    /// A health probe failed without destroying the service.
    #[error("gateway service health probe failed")]
    HealthProbe(#[source] ServiceProbeError),
    /// VM or materializer cleanup did not complete.
    #[error("gateway service cleanup is incomplete")]
    CleanupIncomplete,
}

/// Handle used by the parent supervisor to control one worker.
#[derive(Clone)]
pub struct ServiceInstanceHandle {
    control: Arc<ControlInner>,
    state: watch::Receiver<ServiceWorkerState>,
    diagnostics: watch::Receiver<ServiceDiagnosticsSnapshot>,
    logs: Option<ServiceLogBufferHandle>,
}

struct ControlInner {
    commands: mpsc::Sender<Command>,
    cancellation: CancellationToken,
    failure: Arc<RwLock<Option<GatewayServiceFailure>>>,
}

impl Drop for ControlInner {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl ServiceInstanceHandle {
    /// Requests bounded shutdown of the instance.
    pub fn shutdown(&self) {
        self.control.cancellation.cancel();
    }

    /// Returns the redacted primary failure observed before worker teardown.
    ///
    /// # Panics
    ///
    /// Panics if the worker failure snapshot lock was poisoned by a prior
    /// panic while updating it.
    #[must_use]
    pub fn failure(&self) -> Option<GatewayServiceFailure> {
        self.control
            .failure
            .read()
            .expect("service failure snapshot lock")
            .as_ref()
            .copied()
    }

    /// Returns the latest content-free lifecycle diagnostics snapshot.
    #[must_use]
    pub fn diagnostics(&self) -> ServiceDiagnosticsSnapshot {
        *self.diagnostics.borrow()
    }

    /// Subscribes to content-free lifecycle diagnostics updates.
    #[must_use]
    pub fn subscribe_diagnostics(&self) -> watch::Receiver<ServiceDiagnosticsSnapshot> {
        self.diagnostics.clone()
    }

    /// Returns the bounded application log queue when capture was opted in.
    #[must_use]
    pub fn service_logs(&self) -> Option<ServiceLogBufferHandle> {
        self.logs.clone()
    }

    /// Requests one bounded health probe. Dropping this future does not stop
    /// the worker or abandon VM cleanup.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when the worker is unavailable or the service
    /// is not ready.
    pub async fn health(&self) -> Result<http::StatusCode, ServiceInstanceError> {
        if *self.state.borrow() != ServiceWorkerState::Ready {
            return Err(ServiceInstanceError::NotReady);
        }
        let (reply, response) = oneshot::channel();
        self.control
            .commands
            .send(Command::Health(reply))
            .await
            .map_err(|_| ServiceInstanceError::CleanupIncomplete)?;
        response
            .await
            .map_err(|_| ServiceInstanceError::CleanupIncomplete)?
    }
}

/// A parent-owned worker future and its control handle.
pub struct ServiceInstance {
    launch: GatewayServiceLaunch,
    vm: Arc<dyn VmInstance>,
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    authority: String,
    policy: ServiceInstancePolicy,
    commands: mpsc::Receiver<Command>,
    cancellation: CancellationToken,
    states: watch::Sender<ServiceWorkerState>,
    failure: Arc<RwLock<Option<GatewayServiceFailure>>>,
    diagnostics: watch::Sender<ServiceDiagnosticsSnapshot>,
    logs: Option<ServiceLogBufferHandle>,
}

/// Creates a worker for one exact, already-provisioned service VM.
///
/// The returned worker must be run by the parent supervisor. No lifecycle task
/// is spawned here, so cancellation and cleanup remain owned by that parent.
/// The parent should retain its own clone of `vm` until the worker reports
/// successful cleanup; a durable VM identifier alone cannot recover a failed
/// destroy while the provider's same-process live-handle registry still owns
/// the instance.
///
/// # Errors
///
/// Returns an error when identity, service, authority, or policy validation
/// fails.
pub fn new_service_instance(
    launch: GatewayServiceLaunch,
    vm: Arc<dyn VmInstance>,
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    authority: impl Into<String>,
    policy: ServiceInstancePolicy,
) -> Result<
    (
        ServiceInstanceHandle,
        watch::Receiver<ServiceWorkerState>,
        ServiceInstance,
    ),
    ServiceInstanceError,
> {
    policy.validate()?;
    launch
        .service
        .validate()
        .map_err(|_| ServiceInstanceError::InvalidPolicy)?;
    let expected = format!("gateway-service-{}", launch.identity.instance_id);
    if launch.identity.instance_id.is_nil()
        || launch.identity.gateway_id.is_nil()
        || launch.identity.revision_id.is_nil()
        || launch.spec.id.0 != expected
        || vm.id().0 != launch.spec.id.0
    {
        return Err(ServiceInstanceError::InvalidPolicy);
    }
    let authority = authority.into();
    if http::HeaderValue::try_from(authority.as_str()).is_err() || authority.is_empty() {
        return Err(ServiceInstanceError::InvalidPolicy);
    }
    let (commands, receiver) = mpsc::channel(COMMAND_CAPACITY);
    let cancellation = CancellationToken::new();
    let (states, state_receiver) = watch::channel(ServiceWorkerState::Provisioned);
    let failure = Arc::new(RwLock::new(None));
    let (diagnostics, diagnostics_receiver) = watch::channel(ServiceDiagnosticsSnapshot::default());
    let logs = matches!(
        launch.service.log_capture_mode,
        ServiceLogCaptureMode::Application
    )
    .then(ServiceLogBufferHandle::new);
    let handle = ServiceInstanceHandle {
        control: Arc::new(ControlInner {
            commands,
            cancellation: cancellation.clone(),
            failure: Arc::clone(&failure),
        }),
        state: state_receiver.clone(),
        diagnostics: diagnostics_receiver,
        logs: logs.clone(),
    };
    let worker = ServiceInstance {
        launch,
        vm,
        resolver,
        authority,
        policy,
        commands: receiver,
        cancellation,
        states,
        failure,
        diagnostics,
        logs,
    };
    Ok((handle, state_receiver, worker))
}

enum Command {
    Health(oneshot::Sender<Result<http::StatusCode, ServiceInstanceError>>),
}

#[path = "service_instance/helpers.rs"]
mod helpers;
#[path = "service_instance/lifecycle.rs"]
mod lifecycle;
#[path = "service_instance/serve.rs"]
mod serve;

#[cfg(test)]
#[path = "service_instance/tests.rs"]
mod tests;

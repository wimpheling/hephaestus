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

impl ServiceInstance {
    /// Runs the VM lifecycle until shutdown, exit, or cleanup failure.
    ///
    /// # Errors
    ///
    /// Returns a redacted lifecycle or cleanup error. A successful return means
    /// that provider and materializer cleanup both completed.
    pub async fn run(mut self) -> Result<(), ServiceInstanceError> {
        let mut events = self.vm.subscribe_events();
        let mut diagnostics = ServiceDiagnostics::new(self.diagnostics.clone());
        let logs = self.logs.clone();
        let mut events_open = true;
        let mut lifecycle = Box::pin(self.run_lifecycle());
        loop {
            tokio::select! {
                biased;
                result = &mut lifecycle => {
                    drop(lifecycle);
                    drain_events(&mut events, &mut diagnostics, &mut events_open, logs.as_ref());
                    return result;
                }
                event = events.recv(), if events_open => {
                    events_open = observe_event(&mut diagnostics, logs.as_ref(), event);
                }
            }
        }
    }

    async fn run_lifecycle(&mut self) -> Result<(), ServiceInstanceError> {
        let startup_deadline = Instant::now()
            .checked_add(self.policy.startup_timeout)
            .ok_or(ServiceInstanceError::InvalidPolicy)?;
        self.set_state(ServiceWorkerState::Starting);
        match time::timeout_at(startup_deadline, self.start_vm()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                let reason = if self.cancellation.is_cancelled() {
                    ServiceInstanceError::Shutdown
                } else {
                    ServiceInstanceError::StartupFailed
                };
                let failure = failure_for_startup(&reason);
                return self
                    .finish_with_failure(FailureOutcome {
                        error: reason,
                        report: failure,
                    })
                    .await;
            }
            Err(_) => {
                return self
                    .finish_with_failure(FailureOutcome::startup_timeout())
                    .await;
            }
        }
        self.set_state(ServiceWorkerState::Probing);
        match self.await_readiness(startup_deadline).await {
            Ok(()) => self.set_state(ServiceWorkerState::Ready),
            Err(outcome) => return self.finish_with_failure(outcome).await,
        }
        self.serve_ready().await
    }

    fn set_state(&self, state: ServiceWorkerState) {
        let _ = self.states.send(state);
        self.diagnostics.send_modify(|snapshot| {
            snapshot.worker_state = state;
        });
        tracing::debug!(
            gateway_id = %self.launch.identity.gateway_id,
            revision_id = %self.launch.identity.revision_id,
            instance_id = %self.launch.identity.instance_id,
            state = ?state,
            "gateway service lifecycle state changed"
        );
    }

    async fn start_vm(&self) -> Result<(), VmError> {
        tokio::select! {
            () = self.cancellation.cancelled() => Err(VmError::InvalidState("service cancelled")),
            result = self.vm.start() => result,
        }
    }

    async fn await_readiness(&self, deadline: Instant) -> Result<(), FailureOutcome> {
        let mut wait = Box::pin(self.vm.wait());
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FailureOutcome::readiness(
                    ServiceInstanceError::StartupTimeout,
                ));
            }
            let probe = time::timeout_at(
                deadline,
                probe_private_service_http(
                    self.vm.as_ref(),
                    &self.launch.service.readiness_path,
                    &self.authority,
                    ServiceProbePolicy::new(self.policy.probe_timeout.min(remaining)),
                ),
            );
            tokio::pin!(probe);
            tokio::select! {
                () = self.cancellation.cancelled() => return Err(FailureOutcome::shutdown()),
                exit = &mut wait => return Err(map_exit(&exit)),
                result = &mut probe => match result {
                    Err(_) => return Err(FailureOutcome::readiness(ServiceInstanceError::StartupTimeout)),
                    Ok(Ok(_)) => return Ok(()),
                    Ok(Err(ServiceProbeError::InvalidPolicy | ServiceProbeError::InvalidAuthority | ServiceProbeError::Contract)) => return Err(FailureOutcome::readiness(ServiceInstanceError::StartupFailed)),
                    Ok(Err(_)) => {}
                },
            }
            let pause = self
                .policy
                .probe_interval
                .min(deadline.saturating_duration_since(Instant::now()));
            if pause.is_zero() {
                return Err(FailureOutcome::readiness(
                    ServiceInstanceError::StartupTimeout,
                ));
            }
            tokio::select! {
                () = self.cancellation.cancelled() => return Err(FailureOutcome::shutdown()),
                exit = &mut wait => return Err(map_exit(&exit)),
                () = time::sleep(pause) => {}
            }
        }
    }

    async fn serve_ready(&mut self) -> Result<(), ServiceInstanceError> {
        let mut wait = Box::pin(self.vm.wait());
        loop {
            tokio::select! {
                () = self.cancellation.cancelled() => return self.finish_with_failure(FailureOutcome::shutdown()).await,
                exit = &mut wait => return self.finish_with_failure(map_exit(&exit)).await,
                command = self.commands.recv() => match command {
                    Some(Command::Health(reply)) => {
                        let probe = self.health_probe();
                        tokio::pin!(probe);
                        tokio::select! {
                            () = self.cancellation.cancelled() => return self.finish_with_failure(FailureOutcome::shutdown()).await,
                            exit = &mut wait => return self.finish_with_failure(map_exit(&exit)).await,
                            result = &mut probe => { let _ = reply.send(result); }
                        }
                    }
                    None => return self.finish_with_failure(FailureOutcome::shutdown()).await,
                },
            }
        }
    }

    async fn health_probe(&self) -> Result<http::StatusCode, ServiceInstanceError> {
        probe_private_service_http(
            self.vm.as_ref(),
            &self.launch.service.health_path,
            &self.authority,
            ServiceProbePolicy::new(self.policy.probe_timeout),
        )
        .await
        .map(|success| success.status)
        .map_err(ServiceInstanceError::HealthProbe)
    }

    async fn finish_with_failure(
        &self,
        outcome: FailureOutcome,
    ) -> Result<(), ServiceInstanceError> {
        if let Some(failure) = outcome.report {
            *self.failure.write().expect("service failure snapshot lock") = Some(failure);
        }
        let primary = outcome.error;
        self.set_state(ServiceWorkerState::Stopping);
        let stop = time::timeout(
            self.policy.shutdown_timeout,
            self.vm.stop(StopMode::Graceful {
                timeout: self.policy.shutdown_timeout,
            }),
        )
        .await;
        let _ = stop;
        let destroyed = time::timeout(self.policy.shutdown_timeout, self.vm.destroy()).await;
        if destroyed.is_err() || destroyed.ok().and_then(Result::err).is_some() {
            self.set_state(ServiceWorkerState::CleanupIncomplete);
            return Err(ServiceInstanceError::CleanupIncomplete);
        }
        let cleaned = time::timeout(
            self.policy.shutdown_timeout,
            self.resolver.cleanup_service_launch(self.launch.identity),
        )
        .await;
        if cleaned.is_err() || cleaned.ok().and_then(Result::err).is_some() {
            self.set_state(ServiceWorkerState::CleanupIncomplete);
            return Err(ServiceInstanceError::CleanupIncomplete);
        }
        self.set_state(ServiceWorkerState::Stopped);
        if matches!(primary, ServiceInstanceError::Shutdown) {
            Ok(())
        } else {
            Err(primary)
        }
    }
}

struct FailureOutcome {
    error: ServiceInstanceError,
    report: Option<GatewayServiceFailure>,
}

impl FailureOutcome {
    const fn shutdown() -> Self {
        Self {
            error: ServiceInstanceError::Shutdown,
            report: None,
        }
    }

    const fn startup_timeout() -> Self {
        Self {
            error: ServiceInstanceError::StartupTimeout,
            report: Some(GatewayServiceFailure {
                code: GatewayServiceFailureCode::Startup,
                exit_code: None,
                exit_signal: None,
            }),
        }
    }

    const fn readiness(error: ServiceInstanceError) -> Self {
        Self {
            error,
            report: Some(GatewayServiceFailure {
                code: GatewayServiceFailureCode::Readiness,
                exit_code: None,
                exit_signal: None,
            }),
        }
    }
}

fn map_exit(result: &Result<VmExit, VmError>) -> FailureOutcome {
    let report = result.as_ref().map_or_else(
        |_| failure_for_code(GatewayServiceFailureCode::UnexpectedExit),
        |exit| {
            GatewayServiceFailure::new(
                GatewayServiceFailureCode::UnexpectedExit,
                exit.code,
                exit.signal,
            )
            .ok()
            .or_else(|| failure_for_code(GatewayServiceFailureCode::UnexpectedExit))
        },
    );
    FailureOutcome {
        error: ServiceInstanceError::UnexpectedExit,
        report,
    }
}

fn failure_for_startup(error: &ServiceInstanceError) -> Option<GatewayServiceFailure> {
    match error {
        ServiceInstanceError::StartupFailed | ServiceInstanceError::StartupTimeout => {
            failure_for_code(GatewayServiceFailureCode::Startup)
        }
        ServiceInstanceError::Shutdown
        | ServiceInstanceError::InvalidPolicy
        | ServiceInstanceError::UnexpectedExit
        | ServiceInstanceError::NotReady
        | ServiceInstanceError::HealthProbe(_)
        | ServiceInstanceError::CleanupIncomplete => None,
    }
}

fn failure_for_code(code: GatewayServiceFailureCode) -> Option<GatewayServiceFailure> {
    GatewayServiceFailure::new(code, None, None).ok()
}

const MAX_DRAIN_EVENTS: usize = 64;

fn capture_log_event(logs: Option<&ServiceLogBufferHandle>, event: &VmEvent) {
    let Some(logs) = logs else {
        return;
    };
    if let VmEvent::Log { stream, bytes } = event {
        logs.try_record(*stream, OffsetDateTime::now_utc(), bytes);
    }
}

fn observe_event(
    diagnostics: &mut ServiceDiagnostics,
    logs: Option<&ServiceLogBufferHandle>,
    event: Result<VmEvent, broadcast::error::RecvError>,
) -> bool {
    match event {
        Ok(event) => {
            capture_log_event(logs, &event);
            diagnostics.observe(Ok(event))
        }
        Err(error @ broadcast::error::RecvError::Lagged(skipped)) => {
            if let Some(logs) = logs {
                logs.record_provider_lag(skipped);
            }
            diagnostics.observe(Err(error))
        }
        Err(error) => diagnostics.observe(Err(error)),
    }
}

fn drain_events(
    events: &mut broadcast::Receiver<vm_trait::VmEvent>,
    diagnostics: &mut ServiceDiagnostics,
    events_open: &mut bool,
    logs: Option<&ServiceLogBufferHandle>,
) {
    for _ in 0..MAX_DRAIN_EVENTS {
        match events.try_recv() {
            Ok(event) => {
                *events_open = observe_event(diagnostics, logs, Ok(event));
            }
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                *events_open = observe_event(
                    diagnostics,
                    logs,
                    Err(broadcast::error::RecvError::Lagged(skipped)),
                );
            }
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(broadcast::error::TryRecvError::Closed) => {
                *events_open =
                    observe_event(diagnostics, logs, Err(broadcast::error::RecvError::Closed));
                break;
            }
        }
        if !*events_open {
            break;
        }
    }
}

enum Command {
    Health(oneshot::Sender<Result<http::StatusCode, ServiceInstanceError>>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GatewayEdgeError, GatewayServiceIdentity};
    use async_trait::async_trait;
    use gateway_domain::ServiceProbePath;
    use std::{
        collections::{BTreeMap, VecDeque},
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
        sync::{Notify, broadcast},
    };
    use uuid::Uuid;
    use vm_trait::{
        BoxedPrivateServiceConnection, GuestCommand, NetworkMode, RootFilesystem, VmId,
        VmResources, VmSpec,
    };

    struct FakeVm {
        id: VmId,
        connections: Mutex<VecDeque<BoxedPrivateServiceConnection>>,
        events: broadcast::Sender<vm_trait::VmEvent>,
        exited: tokio::sync::watch::Sender<Option<VmExit>>,
        first_open: Arc<Notify>,
        starts: AtomicUsize,
        opens: AtomicUsize,
        destroys: AtomicUsize,
        destroy_ok: AtomicBool,
        start_ok: AtomicBool,
        emit_start_log: AtomicBool,
        stop_ok: AtomicBool,
        start_delay: Mutex<Duration>,
    }

    impl FakeVm {
        fn new(id: VmId) -> Arc<Self> {
            let (events, _) = broadcast::channel(2);
            let (exited, _) = tokio::sync::watch::channel(None);
            Arc::new(Self {
                id,
                connections: Mutex::new(VecDeque::new()),
                events,
                exited,
                first_open: Arc::new(Notify::new()),
                starts: AtomicUsize::new(0),
                opens: AtomicUsize::new(0),
                destroys: AtomicUsize::new(0),
                destroy_ok: AtomicBool::new(true),
                start_ok: AtomicBool::new(true),
                emit_start_log: AtomicBool::new(false),
                stop_ok: AtomicBool::new(true),
                start_delay: Mutex::new(Duration::ZERO),
            })
        }

        fn push(&self, connection: BoxedPrivateServiceConnection) {
            self.connections
                .lock()
                .expect("connection lock")
                .push_back(connection);
        }

        fn exit_with(&self, exit: VmExit) {
            let _ = self.exited.send(Some(exit));
        }
    }

    #[async_trait]
    impl VmInstance for FakeVm {
        fn id(&self) -> &VmId {
            &self.id
        }

        async fn start(&self) -> Result<(), VmError> {
            self.starts.fetch_add(1, Ordering::Relaxed);
            if !self.start_ok.load(Ordering::Relaxed) {
                return Err(VmError::Destroyed);
            }
            if self.emit_start_log.load(Ordering::Relaxed) {
                let _ = self.events.send(vm_trait::VmEvent::Log {
                    stream: vm_trait::LogStream::Stdout,
                    bytes: b"before-readiness".to_vec(),
                });
            }
            let _ = self.events.send(vm_trait::VmEvent::Started {
                ingress: Vec::new(),
            });
            let delay = *self.start_delay.lock().expect("start delay lock");
            tokio::time::sleep(delay).await;
            Ok(())
        }

        async fn stop(&self, _: StopMode) -> Result<(), VmError> {
            if self.stop_ok.load(Ordering::Relaxed) {
                Ok(())
            } else {
                Err(VmError::Destroyed)
            }
        }

        async fn wait(&self) -> Result<VmExit, VmError> {
            let mut receiver = self.exited.subscribe();
            loop {
                let current = receiver.borrow().clone();
                if let Some(exit) = current {
                    return Ok(exit);
                }
                receiver.changed().await.map_err(|_| VmError::Destroyed)?;
            }
        }

        async fn open_private_service_connection(
            &self,
        ) -> Result<BoxedPrivateServiceConnection, VmError> {
            if self.opens.fetch_add(1, Ordering::Relaxed) == 0 {
                self.first_open.notify_waiters();
            }
            self.connections
                .lock()
                .expect("connection lock")
                .pop_front()
                .ok_or_else(|| VmError::Unavailable {
                    resource: String::from("fake connection"),
                    reason: String::from("none queued"),
                })
        }

        fn subscribe_events(&self) -> broadcast::Receiver<vm_trait::VmEvent> {
            self.events.subscribe()
        }

        async fn destroy(&self) -> Result<(), VmError> {
            self.destroys.fetch_add(1, Ordering::Relaxed);
            if self.destroy_ok.load(Ordering::Relaxed) {
                Ok(())
            } else {
                Err(VmError::Destroyed)
            }
        }
    }

    struct FakeResolver {
        cleanups: AtomicUsize,
    }

    #[async_trait]
    impl GatewayServiceLaunchResolver for FakeResolver {
        async fn resolve_service_launch(
            &self,
            _: crate::GatewayServiceLaunchRequest,
        ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
            Err(GatewayEdgeError::Unavailable)
        }

        async fn cleanup_service_launch(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<(), GatewayEdgeError> {
            self.cleanups.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn launch(instance_id: Uuid) -> GatewayServiceLaunch {
        let identity = GatewayServiceIdentity {
            instance_id,
            gateway_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        };
        let ready = ServiceProbePath::parse("/readyz").expect("ready path");
        let health = ServiceProbePath::parse("/healthz").expect("health path");
        GatewayServiceLaunch {
            identity,
            service: gateway_domain::GatewayServiceConfig::new(8080, ready, health)
                .expect("service"),
            spec: VmSpec {
                id: VmId(format!("gateway-service-{}", identity.instance_id)),
                root: RootFilesystem::Directory {
                    host_path: PathBuf::from("/fake/root"),
                },
                disks: Vec::new(),
                mounts: Vec::new(),
                resources: VmResources {
                    vcpus: 1,
                    memory_mib: 64,
                },
                network: NetworkMode::Disabled,
                command: GuestCommand {
                    program: String::from("/service"),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    working_dir: None,
                },
                runtime_authority: None,
                runtime_git_bridge: None,
                private_http_service: None,
                labels: BTreeMap::new(),
            },
        }
    }

    fn launch_with_logs(instance_id: Uuid) -> GatewayServiceLaunch {
        let mut launch = launch(instance_id);
        launch.service = launch
            .service
            .with_log_capture_mode(gateway_domain::ServiceLogCaptureMode::Application);
        launch
    }

    fn policy() -> ServiceInstancePolicy {
        ServiceInstancePolicy::new(
            Duration::from_millis(120),
            Duration::from_millis(1),
            Duration::from_millis(30),
            Duration::from_millis(50),
        )
    }

    async fn response_peer(mut peer: DuplexStream, status: u16) {
        let mut request = [0_u8; 1024];
        let _ = peer.read(&mut request).await;
        let response =
            format!("HTTP/1.1 {status} OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        let _ = peer.write_all(response.as_bytes()).await;
    }

    #[tokio::test]
    async fn application_log_capture_is_opt_in_and_starts_before_readiness() {
        let launch = launch_with_logs(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        vm.emit_start_log.store(true, Ordering::Relaxed);
        let (client, peer) = tokio::io::duplex(4096);
        vm.push(Box::new(client));
        tokio::spawn(response_peer(peer, 200));
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (handle, mut state, worker) =
            new_service_instance(launch, vm, resolver, "service.test", policy()).expect("worker");
        let logs = handle.service_logs().expect("application log queue");
        let task = tokio::spawn(worker.run());
        wait_for_state(&mut state, ServiceWorkerState::Ready).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if logs.snapshot().queued_chunks > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("start log captured");
        let records = logs.drain(8, 1024);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].bytes, b"before-readiness");
        handle.shutdown();
        assert!(task.await.expect("worker join").is_ok());
    }

    #[test]
    fn disabled_log_capture_has_no_raw_queue() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (handle, _, _) =
            new_service_instance(launch, vm, resolver, "service.test", policy()).expect("worker");
        assert!(handle.service_logs().is_none());
    }

    async fn wait_for_state(
        receiver: &mut watch::Receiver<ServiceWorkerState>,
        expected: ServiceWorkerState,
    ) {
        while *receiver.borrow() != expected {
            receiver.changed().await.expect("worker state");
        }
    }

    #[tokio::test]
    async fn readiness_is_required_before_ready_and_cleanup_is_exact() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.push(Box::new(client));
        tokio::spawn(response_peer(peer, 503));
        let (handle, state, worker) = new_service_instance(
            launch,
            vm.clone(),
            resolver.clone(),
            "service.test",
            policy(),
        )
        .expect("worker");
        let task = tokio::spawn(worker.run());
        assert_eq!(handle.health().await, Err(ServiceInstanceError::NotReady));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_ne!(*state.borrow(), ServiceWorkerState::Ready);
        let result = task.await.expect("worker join");
        assert!(matches!(
            result,
            Err(ServiceInstanceError::StartupTimeout | ServiceInstanceError::StartupFailed)
        ));
        assert_eq!(
            handle.failure().map(|failure| failure.code),
            Some(GatewayServiceFailureCode::Readiness)
        );
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
        assert!(handle.diagnostics().lifecycle.started());
        assert_eq!(
            handle.diagnostics().worker_state,
            ServiceWorkerState::Stopped
        );
        drop(handle);
    }

    #[tokio::test]
    async fn ready_health_failure_keeps_instance_alive_until_shutdown() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        for status in [200, 503] {
            let (client, peer) = tokio::io::duplex(4096);
            vm.push(Box::new(client));
            tokio::spawn(response_peer(peer, status));
        }
        let (handle, mut state, worker) = new_service_instance(
            launch,
            vm.clone(),
            resolver.clone(),
            "service.test",
            policy(),
        )
        .expect("worker");
        let task = tokio::spawn(worker.run());
        wait_for_state(&mut state, ServiceWorkerState::Ready).await;
        assert!(matches!(
            handle.health().await,
            Err(ServiceInstanceError::HealthProbe(
                ServiceProbeError::NonSuccess(_)
            ))
        ));
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 0);
        handle.shutdown();
        assert!(task.await.expect("worker join").is_ok());
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn destroy_failure_does_not_claim_materializer_cleanup() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        vm.destroy_ok.store(false, Ordering::Relaxed);
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (handle, mut state, worker) = new_service_instance(
            launch,
            vm.clone(),
            resolver.clone(),
            "service.test",
            policy(),
        )
        .expect("worker");
        let task = tokio::spawn(worker.run());
        wait_for_state(&mut state, ServiceWorkerState::Probing).await;
        drop(handle);
        assert_eq!(
            task.await.expect("worker join"),
            Err(ServiceInstanceError::CleanupIncomplete)
        );
        assert_eq!(*state.borrow(), ServiceWorkerState::CleanupIncomplete);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn startup_deadline_includes_blocked_start() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        *vm.start_delay.lock().expect("start delay lock") = Duration::from_millis(200);
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let short = ServiceInstancePolicy::new(
            Duration::from_millis(20),
            Duration::from_millis(1),
            Duration::from_millis(10),
            Duration::from_millis(50),
        );
        let (handle, _, worker) =
            new_service_instance(launch, vm.clone(), resolver.clone(), "service.test", short)
                .expect("worker");
        let task = tokio::spawn(worker.run());
        assert_eq!(
            task.await.expect("worker join"),
            Err(ServiceInstanceError::StartupTimeout)
        );
        assert_eq!(
            handle.failure().map(|failure| failure.code),
            Some(GatewayServiceFailureCode::Startup)
        );
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
        drop(handle);
    }

    #[tokio::test]
    async fn startup_failure_is_classified_as_startup() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        vm.start_ok.store(false, Ordering::Relaxed);
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (handle, _, worker) =
            new_service_instance(launch, vm, resolver, "service.test", policy()).expect("worker");
        assert_eq!(
            tokio::spawn(worker.run()).await.expect("worker join"),
            Err(ServiceInstanceError::StartupFailed)
        );
        assert_eq!(
            handle.failure().map(|failure| failure.code),
            Some(GatewayServiceFailureCode::Startup)
        );
    }

    #[tokio::test]
    async fn dropping_last_handle_cancels_blocked_start() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        *vm.start_delay.lock().expect("start delay lock") = Duration::from_millis(200);
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (handle, _, worker) = new_service_instance(
            launch,
            vm.clone(),
            resolver.clone(),
            "service.test",
            policy(),
        )
        .expect("worker");
        let task = tokio::spawn(worker.run());
        tokio::time::sleep(Duration::from_millis(5)).await;
        drop(handle);
        assert!(task.await.expect("worker join").is_ok());
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn cancellation_wins_over_a_busy_event_channel() {
        let launch = launch_with_logs(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        *vm.start_delay.lock().expect("start delay lock") = Duration::from_millis(200);
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (handle, _, worker) = new_service_instance(
            launch,
            vm.clone(),
            resolver.clone(),
            "service.test",
            policy(),
        )
        .expect("worker");
        let mut diagnostics = handle.subscribe_diagnostics();
        let logs = handle.service_logs().expect("application log queue");
        let mut task = tokio::spawn(worker.run());
        let events = vm.events.clone();
        let stop_flood = Arc::new(AtomicBool::new(false));
        let flood_stop = Arc::clone(&stop_flood);
        let mut flood = tokio::spawn(async move {
            while !flood_stop.load(Ordering::Relaxed) {
                for _ in 0..100 {
                    let _ = events.send(vm_trait::VmEvent::Log {
                        stream: vm_trait::LogStream::Stdout,
                        bytes: b"request Authorization secret".to_vec(),
                    });
                    tokio::task::yield_now().await;
                }
            }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = *diagnostics.borrow();
                if snapshot.lifecycle.started() || snapshot.stdout_bytes > 0 {
                    break;
                }
                diagnostics.changed().await.expect("diagnostics update");
            }
        })
        .await
        .expect("event flood observed");
        drop(handle);
        let worker_result =
            if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                result.expect("worker join")
            } else {
                task.abort();
                let _ = task.await;
                panic!("worker cleanup timed out");
            };
        assert!(worker_result.is_ok());
        stop_flood.store(true, Ordering::Relaxed);
        if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut flood).await {
            result.expect("event flood");
        } else {
            flood.abort();
            let _ = flood.await;
            panic!("event flood did not stop");
        }
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
        assert!(logs.snapshot().queued_chunks > 0);
    }

    #[tokio::test]
    async fn event_flood_does_not_restart_a_blocked_readiness_probe() {
        let launch = launch_with_logs(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        let (client, _peer) = tokio::io::duplex(4096);
        vm.push(Box::new(client));
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let slow_policy = ServiceInstancePolicy::new(
            Duration::from_secs(2),
            Duration::from_millis(20),
            Duration::from_millis(500),
            Duration::from_millis(50),
        );
        let (handle, _, worker) = new_service_instance(
            launch,
            vm.clone(),
            resolver.clone(),
            "service.test",
            slow_policy,
        )
        .expect("worker");
        let logs = handle.service_logs().expect("application log queue");
        let first_open = vm.first_open.clone();
        let opened = first_open.notified();
        let mut diagnostics = handle.subscribe_diagnostics();
        let mut task = tokio::spawn(worker.run());
        tokio::time::timeout(Duration::from_secs(2), opened)
            .await
            .expect("readiness probe opened");
        assert_eq!(vm.opens.load(Ordering::Relaxed), 1);
        let events = vm.events.clone();
        let stop_flood = Arc::new(AtomicBool::new(false));
        let flood_stop = Arc::clone(&stop_flood);
        let mut flood = tokio::spawn(async move {
            while !flood_stop.load(Ordering::Relaxed) {
                for _ in 0..100 {
                    let _ = events.send(vm_trait::VmEvent::Log {
                        stream: vm_trait::LogStream::Stdout,
                        bytes: b"request Authorization secret".to_vec(),
                    });
                    tokio::task::yield_now().await;
                }
            }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = *diagnostics.borrow();
                if snapshot.stdout_bytes > 0 || snapshot.lagged_events > 0 {
                    break;
                }
                diagnostics.changed().await.expect("diagnostics update");
            }
        })
        .await
        .expect("event flood consumed");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if logs.snapshot().queued_chunks > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("application log flood captured");
        assert_eq!(vm.opens.load(Ordering::Relaxed), 1);
        handle.shutdown();
        let worker_result =
            if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                result.expect("worker join")
            } else {
                task.abort();
                let _ = task.await;
                panic!("worker cleanup timed out");
            };
        assert!(worker_result.is_ok());
        stop_flood.store(true, Ordering::Relaxed);
        if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut flood).await {
            result.expect("event flood");
        } else {
            flood.abort();
            let _ = flood.await;
            panic!("event flood did not stop");
        }
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn stop_failure_still_destroys_and_cleans_materializer() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        vm.stop_ok.store(false, Ordering::Relaxed);
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.push(Box::new(client));
        tokio::spawn(response_peer(peer, 200));
        let (handle, mut state, worker) = new_service_instance(
            launch,
            vm.clone(),
            resolver.clone(),
            "service.test",
            policy(),
        )
        .expect("worker");
        let task = tokio::spawn(worker.run());
        wait_for_state(&mut state, ServiceWorkerState::Ready).await;
        handle.shutdown();
        assert!(task.await.expect("worker join").is_ok());
        assert_eq!(handle.failure(), None);
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn vm_exit_triggers_cleanup_after_readiness() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.push(Box::new(client));
        tokio::spawn(response_peer(peer, 200));
        let (handle, mut state, worker) = new_service_instance(
            launch,
            vm.clone(),
            resolver.clone(),
            "service.test",
            policy(),
        )
        .expect("worker");
        let task = tokio::spawn(worker.run());
        wait_for_state(&mut state, ServiceWorkerState::Ready).await;
        vm.exit_with(VmExit {
            code: Some(17),
            signal: None,
        });
        assert_eq!(
            task.await.expect("worker join"),
            Err(ServiceInstanceError::UnexpectedExit)
        );
        assert_eq!(
            handle.failure(),
            Some(GatewayServiceFailure {
                code: GatewayServiceFailureCode::UnexpectedExit,
                exit_code: Some(17),
                exit_signal: None,
            })
        );
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
        drop(handle);
    }

    #[tokio::test]
    async fn exit_signal_is_retained_when_destroy_fails() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        vm.destroy_ok.store(false, Ordering::Relaxed);
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.push(Box::new(client));
        tokio::spawn(response_peer(peer, 200));
        let (handle, mut state, worker) =
            new_service_instance(launch, vm.clone(), resolver, "service.test", policy())
                .expect("worker");
        let task = tokio::spawn(worker.run());
        wait_for_state(&mut state, ServiceWorkerState::Ready).await;
        vm.exit_with(VmExit {
            code: None,
            signal: Some(9),
        });
        assert_eq!(
            task.await.expect("worker join"),
            Err(ServiceInstanceError::CleanupIncomplete)
        );
        assert_eq!(
            handle.failure(),
            Some(GatewayServiceFailure {
                code: GatewayServiceFailureCode::UnexpectedExit,
                exit_code: None,
                exit_signal: Some(9),
            })
        );
    }

    #[tokio::test]
    async fn malformed_exit_metadata_is_redacted() {
        let launch = launch(Uuid::new_v4());
        let vm = FakeVm::new(launch.spec.id.clone());
        let resolver = Arc::new(FakeResolver {
            cleanups: AtomicUsize::new(0),
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.push(Box::new(client));
        tokio::spawn(response_peer(peer, 200));
        let (handle, mut state, worker) =
            new_service_instance(launch, vm.clone(), resolver, "service.test", policy())
                .expect("worker");
        let task = tokio::spawn(worker.run());
        wait_for_state(&mut state, ServiceWorkerState::Ready).await;
        vm.exit_with(VmExit {
            code: Some(999),
            signal: Some(9),
        });
        assert_eq!(
            task.await.expect("worker join"),
            Err(ServiceInstanceError::UnexpectedExit)
        );
        assert_eq!(
            handle.failure(),
            Some(GatewayServiceFailure {
                code: GatewayServiceFailureCode::UnexpectedExit,
                exit_code: None,
                exit_signal: None,
            })
        );
    }
}

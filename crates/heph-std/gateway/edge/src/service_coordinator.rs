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

impl GatewayServiceCoordinator {
    /// Creates a coordinator from an already claimed provisioning lease.
    ///
    /// Both deadlines must be captured before their respective blocking
    /// operations. The outer supervisor must retain and join [`Self::run`].
    ///
    /// # Errors
    ///
    /// Returns an error when the lease, owner, authority, or deadline policy
    /// is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lease: GatewayServiceInstanceLease,
        owner: GatewayServiceOwner,
        initial_lease_deadline: Instant,
        startup_deadline: Instant,
        intent: GatewayServiceStartupIntent,
        ownership: Arc<dyn GatewayServiceOwnership>,
        failure_store: Arc<dyn GatewayServiceFailureStore>,
        resolver: Arc<dyn GatewayServiceLaunchResolver>,
        provider: Arc<dyn VmProvider>,
        targets: Arc<dyn GatewayServiceTargetStore>,
        registry: GatewayServiceRegistry,
        service_authority: impl Into<String>,
        supervisor_policy: GatewayServiceSupervisorPolicy,
    ) -> Result<(Self, GatewayServiceCoordinatorControl), GatewayServiceCoordinatorError> {
        let expected_vm = format!("gateway-service-{}", lease.identity.instance_id);
        if lease.state != GatewayServiceInstanceState::Provisioning
            || lease.identity.instance_id.is_nil()
            || lease.identity.gateway_id.is_nil()
            || lease.identity.revision_id.is_nil()
            || lease.fencing_token <= 0
            || lease.vm_id != expected_vm
            || startup_deadline <= Instant::now()
        {
            return Err(GatewayServiceCoordinatorError::InvalidIdentity);
        }
        owner
            .validate()
            .map_err(|_| GatewayServiceCoordinatorError::InvalidIdentity)?;
        supervisor_policy
            .validate()
            .map_err(|_| GatewayServiceCoordinatorError::InvalidLeasePolicy)?;
        let service_authority = service_authority.into();
        if service_authority.is_empty()
            || http::HeaderValue::try_from(service_authority.as_str()).is_err()
        {
            return Err(GatewayServiceCoordinatorError::InvalidAuthority);
        }
        let (lease_monitor, lease_control) = GatewayServiceLeaseMonitor::new(
            Arc::clone(&ownership),
            lease.clone(),
            owner.clone(),
            supervisor_policy.lease,
            initial_lease_deadline,
        )
        .map_err(|_| GatewayServiceCoordinatorError::InvalidLeasePolicy)?;
        let cancellation = CancellationToken::new();
        let (drain, drain_requested) = watch::channel(false);
        let (status, _) = watch::channel(GatewayServiceCoordinatorStatus::Preparing);
        let control = GatewayServiceCoordinatorControl {
            cancellation: cancellation.clone(),
            drain: drain.clone(),
            status: status.clone(),
        };
        Ok((
            Self {
                lease,
                owner,
                intent,
                ownership,
                failure_store,
                resolver,
                provider,
                targets,
                registry,
                service_authority,
                supervisor_policy,
                startup_deadline,
                lease_control,
                lease_monitor: Some(lease_monitor),
                log_writer: None,
                cancellation,
                drain,
                drain_requested,
                status,
            },
            control,
        ))
    }

    /// Attaches an optional parent-owned durable application-log writer.
    #[must_use]
    pub fn with_log_writer(mut self, config: GatewayServiceLogWriterConfig) -> Self {
        self.log_writer = Some(config);
        self
    }

    /// Runs startup, readiness, registration, promotion, and explicit cleanup.
    ///
    /// # Errors
    ///
    /// Returns a failure retaining any VM or materialization that could not be
    /// cleaned, together with durable cleanup confirmation flags.
    pub async fn run(mut self) -> Result<(), GatewayServiceCoordinatorFailure> {
        let Some(monitor) = self.lease_monitor.take() else {
            return Err(self.failure(
                self.lease.clone(),
                FailureReason::Runtime,
                None,
                false,
                false,
                false,
            ));
        };
        let mut monitor: MonitorFuture = Box::pin(monitor.run());
        let mut monitor_done = false;
        let mut status = self.lease_control.subscribe();
        let mut drain_requested = self.drain_requested.clone();
        let result = self
            .run_inner(
                &mut monitor,
                &mut monitor_done,
                &mut status,
                &mut drain_requested,
            )
            .await;
        self.lease_control.stop();
        if !monitor_done {
            let _ = monitor.await;
        }
        result
    }

    // This is intentionally one state machine: every transition keeps the
    // monitor, worker, cancellation, and readiness signals in one select set.
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    async fn run_inner(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        lease_status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        drain_requested: &mut watch::Receiver<bool>,
    ) -> Result<(), GatewayServiceCoordinatorFailure> {
        self.set_status(GatewayServiceCoordinatorStatus::Preparing);
        let (preparation_handle, preparation) = new_service_preparation(
            Arc::clone(&self.resolver),
            Arc::clone(&self.provider),
            GatewayServiceLaunchRequest {
                identity: self.lease.identity,
            },
        );
        let mut preparation = Box::pin(preparation.run());
        let preparation_result = loop {
            tokio::select! {
                result = &mut preparation => break PrepExit::Complete(result),
                () = self.cancellation.cancelled() => {
                    preparation_handle.cancel();
                    break PrepExit::Cancelled(await_with_monitor(&mut preparation, monitor, monitor_done, lease_status).await);
                }
                () = time::sleep_until(self.startup_deadline) => {
                    preparation_handle.cancel();
                    break PrepExit::StartupDeadline(await_with_monitor(&mut preparation, monitor, monitor_done, lease_status).await);
                }
                signal = monitor_signal(monitor, *monitor_done) => {
                    let _ = signal;
                    *monitor_done = true;
                    preparation_handle.cancel();
                    break PrepExit::LeaseLost(preparation.await);
                }
                changed = lease_status.changed() => {
                    if changed.is_err() || !lease_active(lease_status) {
                        preparation_handle.cancel();
                        break PrepExit::LeaseLost(await_with_monitor(&mut preparation, monitor, monitor_done, lease_status).await);
                    }
                }
            }
        };
        let preparation_reason = preparation_result.reason();
        let prepared = match preparation_result.result() {
            Ok(prepared)
                if Instant::now() < self.startup_deadline && preparation_reason.is_none() =>
            {
                prepared
            }
            Ok(prepared) => {
                return self
                    .cleanup_prepared(
                        monitor,
                        monitor_done,
                        lease_status,
                        prepared,
                        preparation_reason.unwrap_or(FailureReason::StartupDeadline),
                    )
                    .await;
            }
            Err(failure) => {
                let reason = preparation_reason.unwrap_or_else(|| {
                    if Instant::now() >= self.startup_deadline {
                        FailureReason::StartupDeadline
                    } else {
                        FailureReason::Preparation
                    }
                });
                return self
                    .cleanup_preparation_failure(
                        monitor,
                        monitor_done,
                        lease_status,
                        failure,
                        reason,
                    )
                    .await;
            }
        };

        let Some(mut lease) = current_lease(lease_status) else {
            return self
                .cleanup_prepared(
                    monitor,
                    monitor_done,
                    lease_status,
                    prepared,
                    FailureReason::LeaseLost,
                )
                .await;
        };
        self.set_status(GatewayServiceCoordinatorStatus::Starting);
        lease = match self
            .transition(
                monitor,
                monitor_done,
                lease_status,
                self.ownership.mark_starting(&lease, &self.owner),
                true,
            )
            .await
        {
            Ok(lease) => lease,
            Err(reason) => {
                return self
                    .cleanup_prepared(monitor, monitor_done, lease_status, prepared, reason)
                    .await;
            }
        };

        let remaining = self
            .startup_deadline
            .saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return self
                .cleanup_prepared(
                    monitor,
                    monitor_done,
                    lease_status,
                    prepared,
                    FailureReason::StartupDeadline,
                )
                .await;
        }
        let worker_policy = ServiceInstancePolicy::new(
            remaining.min(self.supervisor_policy.instance.startup_timeout),
            self.supervisor_policy.instance.probe_interval,
            self.supervisor_policy.instance.probe_timeout,
            self.supervisor_policy.instance.shutdown_timeout,
        );
        let vm = prepared.vm.clone();
        let Ok((worker_handle, mut worker_state, worker)) = new_service_instance(
            prepared.launch,
            prepared.vm,
            Arc::clone(&self.resolver),
            self.service_authority.clone(),
            worker_policy,
        ) else {
            return self
                .cleanup_vm(
                    monitor,
                    monitor_done,
                    lease_status,
                    lease,
                    vm,
                    FailureReason::Runtime,
                )
                .await;
        };
        let log_writer = match (self.log_writer.clone(), worker_handle.service_logs()) {
            (Some(config), Some(buffer)) => {
                if let Ok(writer) = ServiceLogWriter::new(
                    buffer,
                    config.store,
                    lease.clone(),
                    self.owner.clone(),
                    config.policy,
                ) {
                    Some(writer)
                } else {
                    tracing::warn!(
                        gateway_id = %lease.identity.gateway_id,
                        revision_id = %lease.identity.revision_id,
                        instance_id = %lease.identity.instance_id,
                        "disabled invalid service log writer"
                    );
                    None
                }
            }
            _ => None,
        };
        let mut worker: WorkerFuture = Box::pin(worker.run());
        if let Some(log_writer) = log_writer {
            worker = Box::pin(drive_worker_with_logs(worker, log_writer));
        }
        self.set_status(GatewayServiceCoordinatorStatus::Probing);
        let mut worker_result = None;
        loop {
            if *worker_state.borrow() == ServiceWorkerState::Ready {
                break;
            }
            tokio::select! {
                result = &mut worker => { worker_result = Some(result); break; }
                () = self.cancellation.cancelled() => break,
                () = time::sleep_until(self.startup_deadline) => break,
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; break; }
                changed = worker_state.changed() => if changed.is_err() { break; },
                changed = lease_status.changed() => if changed.is_err() || !lease_active(lease_status) { break; },
            }
        }
        if worker_result.is_some()
            || *worker_state.borrow() != ServiceWorkerState::Ready
            || Instant::now() >= self.startup_deadline
            || !lease_active(lease_status)
            || *monitor_done
        {
            let reason = if self.cancellation.is_cancelled() {
                FailureReason::Cancelled
            } else if Instant::now() >= self.startup_deadline {
                FailureReason::StartupDeadline
            } else if !lease_active(lease_status) || *monitor_done {
                FailureReason::LeaseLost
            } else {
                FailureReason::Runtime
            };
            return self
                .settle_worker(
                    monitor,
                    monitor_done,
                    lease_status,
                    &lease,
                    &worker_handle,
                    &mut worker,
                    &mut worker_result,
                    vm,
                    None,
                    reason,
                )
                .await;
        }

        let key = GatewayServiceInstanceKey {
            identity: lease.identity,
            fencing_token: lease.fencing_token,
        };
        if self
            .registry
            .register(key, vm.clone(), worker_state.clone())
            .is_err()
        {
            return self
                .settle_worker(
                    monitor,
                    monitor_done,
                    lease_status,
                    &lease,
                    &worker_handle,
                    &mut worker,
                    &mut worker_result,
                    vm,
                    None,
                    FailureReason::Runtime,
                )
                .await;
        }
        lease = match self
            .transition_with_worker(
                monitor,
                monitor_done,
                lease_status,
                &mut worker,
                &mut worker_state,
                &mut worker_result,
                self.ownership.mark_ready(&lease, &self.owner),
                true,
            )
            .await
        {
            Ok(lease) => lease,
            Err(reason) => {
                return self
                    .settle_worker(
                        monitor,
                        monitor_done,
                        lease_status,
                        &lease,
                        &worker_handle,
                        &mut worker,
                        &mut worker_result,
                        vm,
                        Some(key),
                        reason,
                    )
                    .await;
            }
        };
        match self.intent {
            GatewayServiceStartupIntent::ActivateDesired => {
                if let Err(reason) = self
                    .transition_with_worker(
                        monitor,
                        monitor_done,
                        lease_status,
                        &mut worker,
                        &mut worker_state,
                        &mut worker_result,
                        self.ownership.promote_ready(&lease, &self.owner),
                        true,
                    )
                    .await
                {
                    return self
                        .settle_worker(
                            monitor,
                            monitor_done,
                            lease_status,
                            &lease,
                            &worker_handle,
                            &mut worker,
                            &mut worker_result,
                            vm,
                            Some(key),
                            reason,
                        )
                        .await;
                }
                if let Some(updated) = current_lease(lease_status) {
                    lease = updated;
                }
            }
            GatewayServiceStartupIntent::RestoreActive => {
                let target = self
                    .targets
                    .get_service_target(lease.identity.gateway_id, lease.identity.revision_id);
                match self
                    .await_target(
                        monitor,
                        monitor_done,
                        lease_status,
                        &mut worker,
                        &mut worker_state,
                        &mut worker_result,
                        target,
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        return self
                            .settle_worker(
                                monitor,
                                monitor_done,
                                lease_status,
                                &lease,
                                &worker_handle,
                                &mut worker,
                                &mut worker_result,
                                vm,
                                Some(key),
                                FailureReason::TargetUnavailable,
                            )
                            .await;
                    }
                    Err(reason) => {
                        return self
                            .settle_worker(
                                monitor,
                                monitor_done,
                                lease_status,
                                &lease,
                                &worker_handle,
                                &mut worker,
                                &mut worker_result,
                                vm,
                                Some(key),
                                reason,
                            )
                            .await;
                    }
                }
            }
        }
        self.set_status(GatewayServiceCoordinatorStatus::Ready);

        let mut health_timer = Box::pin(time::sleep(self.supervisor_policy.health_interval));
        let mut health_failures = 0_u32;
        loop {
            if *drain_requested.borrow() {
                self.drain.send_replace(false);
                match self
                    .drain_and_settle(
                        monitor,
                        monitor_done,
                        lease_status,
                        &mut worker,
                        &mut worker_state,
                        &mut worker_result,
                        &worker_handle,
                        vm.clone(),
                        key,
                        lease.clone(),
                    )
                    .await
                {
                    DrainOutcome::Conflict => {}
                    DrainOutcome::Settled(result) => return *result,
                }
            }
            tokio::select! {
                result = &mut worker => { worker_result = Some(result); return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::Runtime).await; }
                () = self.cancellation.cancelled() => return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::Cancelled).await,
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::LeaseLost).await; }
                changed = lease_status.changed() => if changed.is_err() || !lease_active(lease_status) { return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::LeaseLost).await; },
                changed = worker_state.changed() => if changed.is_err() || *worker_state.borrow() != ServiceWorkerState::Ready { return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::Runtime).await; },
                changed = drain_requested.changed() => if changed.is_err() { return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::Runtime).await; },
                () = &mut health_timer => {
                    match self.await_health(monitor, monitor_done, lease_status, &mut worker, &mut worker_state, &mut worker_result, &worker_handle).await {
                        Ok(true) => health_failures = 0,
                        Ok(false) => {
                            health_failures = health_failures.saturating_add(1);
                            if health_failures >= self.supervisor_policy.health_failure_threshold {
                                return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::Health).await;
                            }
                        }
                        Err(reason) => return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), reason).await,
                    }
                    health_timer.as_mut().reset(Instant::now() + self.supervisor_policy.health_interval);
                },
            }
        }
    }

    // This state machine deliberately carries every live owner through the
    // drain transition, count query, and teardown boundary.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn drain_and_settle(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        worker: &mut WorkerFuture,
        worker_state: &mut watch::Receiver<ServiceWorkerState>,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        worker_handle: &ServiceInstanceHandle,
        vm: Arc<dyn VmInstance>,
        key: GatewayServiceInstanceKey,
        lease: GatewayServiceInstanceLease,
    ) -> DrainOutcome {
        // The grace period covers the fenced state transition and every later
        // invocation-count wait; it must start before any durable I/O.
        let drain_deadline = Instant::now()
            .checked_add(self.supervisor_policy.drain_timeout)
            .unwrap_or_else(Instant::now);
        let current = current_lease(status).unwrap_or_else(|| lease.clone());
        let draining = self
            .transition_draining(
                monitor,
                monitor_done,
                status,
                worker,
                worker_state,
                worker_result,
                self.ownership.mark_draining(&current, &self.owner),
                drain_deadline,
            )
            .await;
        let lease = match draining {
            Ok(DrainTransition::Started(lease)) => lease,
            Ok(DrainTransition::Conflict) => return DrainOutcome::Conflict,
            Ok(DrainTransition::Deadline) => {
                return DrainOutcome::Settled(Box::new(
                    self.settle_worker(
                        monitor,
                        monitor_done,
                        status,
                        &current,
                        worker_handle,
                        worker,
                        worker_result,
                        vm,
                        Some(key),
                        FailureReason::DrainDeadline,
                    )
                    .await,
                ));
            }
            Err(reason) => {
                return DrainOutcome::Settled(Box::new(
                    self.settle_worker(
                        monitor,
                        monitor_done,
                        status,
                        &lease,
                        worker_handle,
                        worker,
                        worker_result,
                        vm,
                        Some(key),
                        reason,
                    )
                    .await,
                ));
            }
        };
        self.set_status(GatewayServiceCoordinatorStatus::Draining);
        loop {
            match self
                .drain_count(
                    monitor,
                    monitor_done,
                    status,
                    worker,
                    worker_state,
                    worker_result,
                    key,
                    drain_deadline,
                )
                .await
            {
                Ok(DrainCount::Zero) => {
                    return DrainOutcome::Settled(Box::new(
                        self.settle_worker(
                            monitor,
                            monitor_done,
                            status,
                            &lease,
                            worker_handle,
                            worker,
                            worker_result,
                            vm,
                            Some(key),
                            FailureReason::Drained,
                        )
                        .await,
                    ));
                }
                Ok(DrainCount::Active | DrainCount::Unavailable) => {
                    if Instant::now() >= drain_deadline {
                        return DrainOutcome::Settled(Box::new(
                            self.settle_worker(
                                monitor,
                                monitor_done,
                                status,
                                &lease,
                                worker_handle,
                                worker,
                                worker_result,
                                vm,
                                Some(key),
                                FailureReason::DrainDeadline,
                            )
                            .await,
                        ));
                    }
                    let wake = Instant::now()
                        .checked_add(DRAIN_POLL_INTERVAL)
                        .unwrap_or(drain_deadline)
                        .min(drain_deadline);
                    if let Err(reason) = self
                        .wait_drain_poll(monitor, monitor_done, status, worker, worker_result, wake)
                        .await
                    {
                        return DrainOutcome::Settled(Box::new(
                            self.settle_worker(
                                monitor,
                                monitor_done,
                                status,
                                &lease,
                                worker_handle,
                                worker,
                                worker_result,
                                vm,
                                Some(key),
                                reason,
                            )
                            .await,
                        ));
                    }
                }
                Err(reason) => {
                    return DrainOutcome::Settled(Box::new(
                        self.settle_worker(
                            monitor,
                            monitor_done,
                            status,
                            &lease,
                            worker_handle,
                            worker,
                            worker_result,
                            vm,
                            Some(key),
                            reason,
                        )
                        .await,
                    ));
                }
            }
        }
    }

    // These explicit ports keep the worker, lease, and DB wait in one select
    // boundary; splitting them would make cancellation ownership implicit.
    #[allow(clippy::too_many_arguments)]
    async fn transition_draining<T, F>(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        worker: &mut WorkerFuture,
        worker_state: &mut watch::Receiver<ServiceWorkerState>,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        operation: F,
        drain_deadline: Instant,
    ) -> Result<DrainTransition<T>, FailureReason>
    where
        F: Future<Output = Result<T, GatewayServiceOwnershipError>>,
    {
        let Some(lease_deadline) = current_deadline(status) else {
            return Err(FailureReason::LeaseLost);
        };
        let deadline = lease_deadline.min(drain_deadline);
        if deadline <= Instant::now() {
            return Ok(DrainTransition::Deadline);
        }
        let mut operation = Box::pin(time::timeout_at(deadline, operation));
        loop {
            tokio::select! {
                result = &mut operation => match result {
                    Ok(Ok(value)) => return Ok(DrainTransition::Started(value)),
                    Ok(Err(GatewayServiceOwnershipError::Conflict)) => return Ok(DrainTransition::Conflict),
                    Ok(Err(_)) => return Err(FailureReason::Ownership),
                    Err(_) if deadline == drain_deadline => {
                        return Err(FailureReason::DrainDeadline);
                    }
                    Err(_) => return Err(FailureReason::LeaseLost),
                },
                result = &mut *worker => { *worker_result = Some(result); return Err(FailureReason::Runtime); },
                changed = worker_state.changed() => if changed.is_err() || *worker_state.borrow() != ServiceWorkerState::Ready { return Err(FailureReason::Runtime); },
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return Err(FailureReason::LeaseLost); },
                changed = status.changed() => if changed.is_err() || !lease_active(status) { return Err(FailureReason::LeaseLost); },
                () = self.cancellation.cancelled() => return Err(FailureReason::Cancelled),
            }
        }
    }

    // The count query shares every live lifecycle signal with the worker.
    #[allow(clippy::too_many_arguments)]
    async fn drain_count(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        worker: &mut WorkerFuture,
        worker_state: &mut watch::Receiver<ServiceWorkerState>,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        key: GatewayServiceInstanceKey,
        drain_deadline: Instant,
    ) -> Result<DrainCount, FailureReason> {
        let Some(lease_deadline) = current_deadline(status) else {
            return Err(FailureReason::LeaseLost);
        };
        let deadline = lease_deadline.min(drain_deadline);
        let mut operation = Box::pin(time::timeout_at(
            deadline,
            self.targets
                .count_accepted_service_invocations_for_instance(key),
        ));
        loop {
            tokio::select! {
                result = &mut operation => match result {
                    Ok(Ok(0)) => return Ok(DrainCount::Zero),
                    Ok(Ok(_)) => return Ok(DrainCount::Active),
                    Ok(Err(GatewayEdgeError::Unavailable)) => return Ok(DrainCount::Unavailable),
                    Ok(Err(_)) => return Err(FailureReason::Ownership),
                    Err(_) if Instant::now() >= drain_deadline => return Ok(DrainCount::Active),
                    Err(_) => return Err(FailureReason::LeaseLost),
                },
                result = &mut *worker => { *worker_result = Some(result); return Err(FailureReason::Runtime); },
                changed = worker_state.changed() => if changed.is_err() || *worker_state.borrow() != ServiceWorkerState::Ready { return Err(FailureReason::Runtime); },
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return Err(FailureReason::LeaseLost); },
                changed = status.changed() => if changed.is_err() || !lease_active(status) { return Err(FailureReason::LeaseLost); },
                () = self.cancellation.cancelled() => return Err(FailureReason::Cancelled),
            }
        }
    }

    // The bounded poll must continue observing lease and worker termination.
    #[allow(clippy::too_many_arguments)]
    async fn wait_drain_poll(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        worker: &mut WorkerFuture,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        wake: Instant,
    ) -> Result<(), FailureReason> {
        let mut timer = Box::pin(time::sleep_until(wake));
        loop {
            tokio::select! {
                () = &mut timer => return Ok(()),
                result = &mut *worker => { *worker_result = Some(result); return Err(FailureReason::Runtime); },
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return Err(FailureReason::LeaseLost); },
                changed = status.changed() => if changed.is_err() || !lease_active(status) { return Err(FailureReason::LeaseLost); },
                () = self.cancellation.cancelled() => return Err(FailureReason::Cancelled),
            }
        }
    }

    // The target query shares the same worker/lease select boundary as the
    // durable transitions, so all live cancellation state stays explicit.
    #[allow(clippy::too_many_arguments)]
    async fn await_target(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        worker: &mut WorkerFuture,
        worker_state: &mut watch::Receiver<ServiceWorkerState>,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        target: impl Future<
            Output = Result<Option<crate::GatewayServiceOwnedTarget>, crate::GatewayEdgeError>,
        >,
    ) -> Result<bool, FailureReason> {
        let target = self
            .transition_with_worker(
                monitor,
                monitor_done,
                status,
                worker,
                worker_state,
                worker_result,
                async {
                    target
                        .await
                        .map_err(|_| GatewayServiceOwnershipError::Unavailable)
                },
                true,
            )
            .await?;
        Ok(target.is_some_and(|target| {
            target.lifecycle == "enabled"
                && target.active_revision_id == Some(self.lease.identity.revision_id)
                && target.revision.revision_id == self.lease.identity.revision_id
                && target.revision.publication_eligible
                && target.revision.release_state.as_deref() == Some("published")
        }))
    }

    // A health request is one bounded in-flight command. Worker, lease, and
    // cancellation signals remain observed while the worker handles it.
    #[allow(clippy::too_many_arguments)]
    async fn await_health(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        worker: &mut WorkerFuture,
        worker_state: &mut watch::Receiver<ServiceWorkerState>,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        handle: &ServiceInstanceHandle,
    ) -> Result<bool, FailureReason> {
        let health = handle.health();
        let mut health = Box::pin(time::timeout(
            self.supervisor_policy.instance.probe_timeout,
            health,
        ));
        loop {
            tokio::select! {
                result = &mut health => return Ok(matches!(result, Ok(Ok(_)))),
                result = &mut *worker => { *worker_result = Some(result); return Err(FailureReason::Runtime); },
                changed = worker_state.changed() => if changed.is_err() || *worker_state.borrow() != ServiceWorkerState::Ready { return Err(FailureReason::Runtime); },
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return Err(FailureReason::LeaseLost); },
                changed = status.changed() => if changed.is_err() || !lease_active(status) { return Err(FailureReason::LeaseLost); },
                () = self.cancellation.cancelled() => return Err(FailureReason::Cancelled),
            }
        }
    }

    async fn cleanup_prepared(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        prepared: PreparedGatewayService,
        reason: FailureReason,
    ) -> Result<(), GatewayServiceCoordinatorFailure> {
        let identity = prepared.launch.identity;
        let vm = prepared.vm;
        let destroy_vm = Arc::clone(&vm);
        let mut destroy = Box::pin(destroy_vm.destroy());
        let destroyed = await_with_monitor(&mut destroy, monitor, monitor_done, status)
            .await
            .is_ok();
        let physical = if destroyed {
            let mut cleanup = Box::pin(self.resolver.cleanup_service_launch(identity));
            await_with_monitor(&mut cleanup, monitor, monitor_done, status)
                .await
                .is_ok()
        } else {
            false
        };
        let retained_vm = if destroyed { None } else { Some(vm) };
        self.cleanup_durable(
            monitor,
            monitor_done,
            status,
            current_lease(status).unwrap_or_else(|| self.lease.clone()),
            retained_vm,
            physical,
            reason,
            failure_for_reason(reason),
        )
        .await
    }

    async fn cleanup_preparation_failure(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        failure: ServicePreparationFailure,
        reason: FailureReason,
    ) -> Result<(), GatewayServiceCoordinatorFailure> {
        let physical = failure.vm.is_none() && !failure.materialization_owned;
        self.cleanup_durable(
            monitor,
            monitor_done,
            status,
            current_lease(status).unwrap_or_else(|| self.lease.clone()),
            failure.vm,
            physical,
            reason,
            failure_for_reason(reason),
        )
        .await
    }

    async fn cleanup_vm(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        lease: GatewayServiceInstanceLease,
        vm: Arc<dyn VmInstance>,
        reason: FailureReason,
    ) -> Result<(), GatewayServiceCoordinatorFailure> {
        let destroy_vm = Arc::clone(&vm);
        let mut destroy = Box::pin(destroy_vm.destroy());
        let destroyed = await_with_monitor(&mut destroy, monitor, monitor_done, status)
            .await
            .is_ok();
        let physical = if destroyed {
            let mut cleanup = Box::pin(self.resolver.cleanup_service_launch(lease.identity));
            await_with_monitor(&mut cleanup, monitor, monitor_done, status)
                .await
                .is_ok()
        } else {
            false
        };
        self.cleanup_durable(
            monitor,
            monitor_done,
            status,
            lease,
            if destroyed { None } else { Some(vm) },
            physical,
            reason,
            failure_for_reason(reason),
        )
        .await
    }

    // This boundary carries every live resource needed for retryable cleanup.
    #[allow(clippy::too_many_arguments)]
    async fn settle_worker(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        lease: &GatewayServiceInstanceLease,
        handle: &ServiceInstanceHandle,
        worker: &mut WorkerFuture,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        vm: Arc<dyn VmInstance>,
        key: Option<GatewayServiceInstanceKey>,
        reason: FailureReason,
    ) -> Result<(), GatewayServiceCoordinatorFailure> {
        if let Some(key) = key {
            let _ = self.registry.unregister(key);
        }
        self.set_status(GatewayServiceCoordinatorStatus::Stopping);
        let current = current_lease(status).unwrap_or_else(|| lease.clone());
        let stopping = self
            .transition_while_worker(
                monitor,
                monitor_done,
                status,
                worker,
                worker_result,
                self.ownership.mark_stopping(&current, &self.owner),
                false,
            )
            .await;
        handle.shutdown();
        if worker_result.is_none() {
            *worker_result = Some(await_with_monitor(worker, monitor, monitor_done, status).await);
        }
        let result = worker_result.take().expect("worker result set");
        let physical = !matches!(result, Err(ServiceInstanceError::CleanupIncomplete));
        let report = cleanup_report(
            physical,
            handle.failure().or_else(|| failure_for_reason(reason)),
        );
        let retained_vm = if physical { None } else { Some(vm) };
        let Ok(stopping) = stopping else {
            return Err(self.failure_pending(
                current,
                reason,
                retained_vm,
                !physical,
                physical,
                false,
                report,
            ));
        };
        self.finish_durable_cleanup(
            monitor,
            monitor_done,
            status,
            stopping,
            retained_vm,
            physical,
            reason,
            report,
        )
        .await
    }

    // Durable transition and physical ownership must be reported together.
    #[allow(clippy::too_many_arguments)]
    async fn cleanup_durable(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        lease: GatewayServiceInstanceLease,
        vm: Option<Arc<dyn VmInstance>>,
        physical: bool,
        reason: FailureReason,
        report: Option<GatewayServiceFailure>,
    ) -> Result<(), GatewayServiceCoordinatorFailure> {
        self.set_status(GatewayServiceCoordinatorStatus::Stopping);
        let stopping = self
            .transition(
                monitor,
                monitor_done,
                status,
                self.ownership.mark_stopping(&lease, &self.owner),
                false,
            )
            .await;
        let report = cleanup_report(physical, report);
        let Ok(stopping) = stopping else {
            return Err(self.failure_pending(lease, reason, vm, !physical, physical, false, report));
        };
        self.finish_durable_cleanup(
            monitor,
            monitor_done,
            status,
            stopping,
            vm,
            physical,
            reason,
            report,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_durable_cleanup(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        stopping: GatewayServiceInstanceLease,
        vm: Option<Arc<dyn VmInstance>>,
        physical: bool,
        reason: FailureReason,
        report: Option<GatewayServiceFailure>,
    ) -> Result<(), GatewayServiceCoordinatorFailure> {
        let report = cleanup_report(physical, report);
        if let Some(report) = report {
            if self
                .record_failure(monitor, monitor_done, status, &stopping, report)
                .await
                .is_err()
            {
                let latest = current_lease(status).unwrap_or_else(|| stopping.clone());
                return Err(self.failure_pending(
                    latest,
                    reason,
                    vm,
                    !physical,
                    physical,
                    false,
                    Some(report),
                ));
            }
        }
        if !physical {
            let latest = current_lease(status).unwrap_or_else(|| stopping.clone());
            return Err(self.failure(latest, reason, vm, true, false, false));
        }
        let durable = self
            .transition(
                monitor,
                monitor_done,
                status,
                self.ownership.mark_cleaned(&stopping, &self.owner),
                false,
            )
            .await
            .is_ok();
        if !durable && physical {
            let latest = current_lease(status).unwrap_or_else(|| stopping.clone());
            return Err(self.failure(latest, reason, vm, false, true, false));
        }
        self.set_status(GatewayServiceCoordinatorStatus::Stopped);
        if matches!(
            reason,
            FailureReason::Cancelled | FailureReason::Drained | FailureReason::DrainDeadline
        ) {
            Ok(())
        } else {
            let latest = current_lease(status).unwrap_or(stopping);
            Err(self.failure(latest, reason, vm, !physical, physical, durable))
        }
    }

    async fn transition<T, F>(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        operation: F,
        honor_cancel: bool,
    ) -> Result<T, FailureReason>
    where
        F: Future<Output = Result<T, GatewayServiceOwnershipError>>,
    {
        let Some(deadline) = current_deadline(status) else {
            return Err(FailureReason::LeaseLost);
        };
        let mut operation = Box::pin(time::timeout_at(deadline, operation));
        loop {
            tokio::select! {
                result = &mut operation => match result { Ok(Ok(value)) => return Ok(value), Ok(Err(_)) => return Err(FailureReason::Ownership), Err(_) => return Err(FailureReason::LeaseLost) },
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return Err(FailureReason::LeaseLost); },
                changed = status.changed() => if changed.is_err() || !lease_active(status) { return Err(FailureReason::LeaseLost); },
                () = self.cancellation.cancelled(), if honor_cancel => return Err(FailureReason::Cancelled),
            }
        }
    }

    async fn record_failure(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        lease: &GatewayServiceInstanceLease,
        failure: GatewayServiceFailure,
    ) -> Result<(), GatewayServiceFailureStoreError> {
        let Some(deadline) = current_deadline(status) else {
            return Err(GatewayServiceFailureStoreError::StaleLease);
        };
        let mut operation = Box::pin(time::timeout_at(
            deadline,
            self.failure_store
                .record_failure(lease, &self.owner, failure),
        ));
        loop {
            tokio::select! {
                result = &mut operation => match result {
                    Ok(result) => return result,
                    Err(_) => return Err(GatewayServiceFailureStoreError::Unavailable),
                },
                signal = monitor_signal(monitor, *monitor_done) => {
                    let _ = signal;
                    *monitor_done = true;
                    return Err(GatewayServiceFailureStoreError::StaleLease);
                }
                changed = status.changed() => {
                    if changed.is_err() || !lease_active(status) {
                        return Err(GatewayServiceFailureStoreError::StaleLease);
                    }
                }
            }
        }
    }

    // Keep worker and lease completion observable while the durable operation
    // is pending; the durable transition remains the authority boundary.
    #[allow(clippy::too_many_arguments)]
    async fn transition_with_worker<T, F>(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        worker: &mut WorkerFuture,
        worker_state: &mut watch::Receiver<ServiceWorkerState>,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        operation: F,
        honor_cancel: bool,
    ) -> Result<T, FailureReason>
    where
        F: Future<Output = Result<T, GatewayServiceOwnershipError>>,
    {
        let Some(deadline) = current_deadline(status) else {
            return Err(FailureReason::LeaseLost);
        };
        let mut operation = Box::pin(time::timeout_at(deadline, operation));
        loop {
            tokio::select! {
                result = &mut operation => match result { Ok(Ok(value)) => return Ok(value), Ok(Err(_)) => return Err(FailureReason::Ownership), Err(_) => return Err(FailureReason::LeaseLost) },
                result = &mut *worker => { *worker_result = Some(result); return Err(FailureReason::Runtime); },
                changed = worker_state.changed() => if changed.is_err() || *worker_state.borrow() != ServiceWorkerState::Ready { return Err(FailureReason::Runtime); },
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return Err(FailureReason::LeaseLost); },
                changed = status.changed() => if changed.is_err() || !lease_active(status) { return Err(FailureReason::LeaseLost); },
                () = self.cancellation.cancelled(), if honor_cancel => return Err(FailureReason::Cancelled),
            }
        }
    }

    // Teardown keeps the worker and lease monitor observed while the durable
    // stopping transition is pending; worker exit does not cancel stopping.
    #[allow(clippy::too_many_arguments)]
    async fn transition_while_worker<T, F>(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        worker: &mut WorkerFuture,
        worker_result: &mut Option<Result<(), ServiceInstanceError>>,
        operation: F,
        honor_cancel: bool,
    ) -> Result<T, FailureReason>
    where
        F: Future<Output = Result<T, GatewayServiceOwnershipError>>,
    {
        let Some(deadline) = current_deadline(status) else {
            return Err(FailureReason::LeaseLost);
        };
        let mut operation = Box::pin(time::timeout_at(deadline, operation));
        loop {
            tokio::select! {
                result = &mut operation => match result { Ok(Ok(value)) => return Ok(value), Ok(Err(_)) => return Err(FailureReason::Ownership), Err(_) => return Err(FailureReason::LeaseLost) },
                result = &mut *worker, if worker_result.is_none() => { *worker_result = Some(result); },
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return Err(FailureReason::LeaseLost); },
                changed = status.changed() => if changed.is_err() || !lease_active(status) { return Err(FailureReason::LeaseLost); },
                () = self.cancellation.cancelled(), if honor_cancel => return Err(FailureReason::Cancelled),
            }
        }
    }

    fn failure(
        &self,
        lease: GatewayServiceInstanceLease,
        reason: FailureReason,
        vm: Option<Arc<dyn VmInstance>>,
        materialization_owned: bool,
        physical: bool,
        durable: bool,
    ) -> GatewayServiceCoordinatorFailure {
        self.failure_pending(
            lease,
            reason,
            vm,
            materialization_owned,
            physical,
            durable,
            None,
        )
    }

    // The failure result keeps lease, resource, cleanup, and report state
    // together so the parent can retry each incomplete responsibility.
    #[allow(clippy::too_many_arguments)]
    fn failure_pending(
        &self,
        lease: GatewayServiceInstanceLease,
        reason: FailureReason,
        vm: Option<Arc<dyn VmInstance>>,
        materialization_owned: bool,
        physical: bool,
        durable: bool,
        pending_failure: Option<GatewayServiceFailure>,
    ) -> GatewayServiceCoordinatorFailure {
        self.set_status(GatewayServiceCoordinatorStatus::Failed);
        GatewayServiceCoordinatorFailure {
            identity: lease.identity,
            lease,
            reason,
            vm,
            materialization_owned,
            physical_cleanup_complete: physical,
            durable_cleanup_complete: durable,
            pending_failure,
        }
    }

    fn set_status(&self, status: GatewayServiceCoordinatorStatus) {
        let _ = self.status.send(status);
    }
}

enum WorkerLogStep {
    Worker(Result<(), ServiceInstanceError>),
    Log(ServiceLogWriterPoll),
}

async fn select_worker_or_log(worker: &mut WorkerFuture, writer: &mut LogWriter) -> WorkerLogStep {
    tokio::select! {
        result = worker.as_mut() => WorkerLogStep::Worker(result),
        poll = writer.poll() => WorkerLogStep::Log(poll),
    }
}

async fn finish_worker_with_logs(
    mut writer: Option<LogWriter>,
    result: Result<(), ServiceInstanceError>,
) -> Result<(), ServiceInstanceError> {
    if let Some(mut writer) = writer.take() {
        let identity = writer.lease().identity;
        writer.shutdown();
        let deadline = Instant::now()
            .checked_add(DEFAULT_SERVICE_LOG_FINAL_FLUSH_TIMEOUT)
            .unwrap_or_else(Instant::now);
        report_log_flush(identity, writer.final_flush(deadline).await);
    }
    result
}

/// Drives one worker and its optional writer without detaching a lifecycle
/// task. The parent coordinator remains responsible for lease/cancellation
/// signals while this future is selected alongside them.
async fn drive_worker_with_logs(
    mut worker: WorkerFuture,
    mut writer: LogWriter,
) -> Result<(), ServiceInstanceError> {
    let mut next_poll = Box::pin(time::sleep(Duration::ZERO));
    loop {
        tokio::select! {
            result = &mut worker => return finish_worker_with_logs(Some(writer), result).await,
            () = &mut next_poll => {
                match select_worker_or_log(&mut worker, &mut writer).await {
                    WorkerLogStep::Worker(result) => {
                        return finish_worker_with_logs(Some(writer), result).await;
                    }
                    WorkerLogStep::Log(ServiceLogWriterPoll::Terminated { .. }) => {
                        let result = worker.await;
                        return finish_worker_with_logs(Some(writer), result).await;
                    }
                    WorkerLogStep::Log(_) => {
                        next_poll.as_mut().reset(Instant::now() + Duration::from_millis(250));
                    }
                }
            }
        }
    }
}

fn report_log_flush(identity: GatewayServiceIdentity, flush: crate::ServiceLogWriterFlush) {
    if !flush.complete {
        tracing::warn!(
            gateway_id = %identity.gateway_id,
            revision_id = %identity.revision_id,
            instance_id = %identity.instance_id,
            unflushed_chunks = flush.unflushed_chunks,
            unflushed_bytes = flush.unflushed_bytes,
            loss_chunks = flush.unflushed_loss.total_chunks(),
            loss_bytes = flush.unflushed_loss.total_bytes(),
            provider_lagged_events = flush.unflushed_loss.provider_lagged_events,
            terminal = flush.terminal_error.is_some(),
            "service log writer flush incomplete"
        );
    }
}

type FailureReason = GatewayServiceCoordinatorFailureReason;

const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(50);

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

fn failure_for_reason(reason: FailureReason) -> Option<GatewayServiceFailure> {
    let code = match reason {
        FailureReason::Preparation => GatewayServiceFailureCode::Preparation,
        FailureReason::StartupDeadline => GatewayServiceFailureCode::Startup,
        FailureReason::Health => GatewayServiceFailureCode::Health,
        FailureReason::CleanupIncomplete => GatewayServiceFailureCode::Cleanup,
        FailureReason::Runtime
        | FailureReason::Ownership
        | FailureReason::Cancelled
        | FailureReason::LeaseLost
        | FailureReason::TargetUnavailable
        | FailureReason::Drained
        | FailureReason::DrainDeadline => return None,
    };
    GatewayServiceFailure::new(code, None, None).ok()
}

fn cleanup_report(
    physical: bool,
    report: Option<GatewayServiceFailure>,
) -> Option<GatewayServiceFailure> {
    report.or_else(|| {
        if physical {
            None
        } else {
            failure_for_reason(FailureReason::CleanupIncomplete)
        }
    })
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
mod tests {
    use super::*;
    use crate::{
        GatewayEdgeError, GatewayLimits, GatewayRequest, GatewayScheme, GatewayServiceFailure,
        GatewayServiceFailureStoreError, GatewayServiceLaunch, GatewayServiceLeasePolicy,
        ServiceHttpPolicy, ServiceInstancePolicy, TrustedRequestMetadata,
    };
    use async_trait::async_trait;
    use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
    use std::{
        collections::BTreeMap,
        collections::VecDeque,
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
        sync::{Notify, broadcast},
        time::timeout,
    };
    use uuid::Uuid;
    use vm_trait::{
        GuestCommand, NetworkMode, PrivateHttpServiceSpec, RootFilesystem, StopMode, VmError,
        VmEvent, VmExit, VmId, VmResources, VmSpec,
    };

    struct MockResolver {
        launch: GatewayServiceLaunch,
        started: Notify,
        release: Notify,
        cleanups: AtomicUsize,
    }

    #[async_trait]
    impl GatewayServiceLaunchResolver for MockResolver {
        async fn resolve_service_launch(
            &self,
            _: GatewayServiceLaunchRequest,
        ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(self.launch.clone())
        }

        async fn cleanup_service_launch(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<(), GatewayEdgeError> {
            self.cleanups.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    struct MockProvider {
        provisions: AtomicUsize,
        vm: Option<Arc<MockVm>>,
        started: Notify,
        release: Notify,
        blocked: AtomicBool,
    }

    #[async_trait]
    impl VmProvider for MockProvider {
        fn name(&self) -> &'static str {
            "coordinator-test"
        }

        async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
            self.provisions.fetch_add(1, Ordering::Relaxed);
            self.started.notify_one();
            if self.blocked.load(Ordering::Relaxed) {
                self.release.notified().await;
            }
            self.vm
                .clone()
                .map(|vm| vm as Arc<dyn VmInstance>)
                .ok_or_else(|| VmError::Unavailable {
                    resource: String::from("test provider"),
                    reason: String::from("provision must not be reached"),
                })
        }

        async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
            Ok(())
        }
    }

    struct MockVm {
        id: VmId,
        events: broadcast::Sender<VmEvent>,
        connections: Mutex<VecDeque<vm_trait::BoxedPrivateServiceConnection>>,
        starts: AtomicUsize,
        destroys: AtomicUsize,
        destroy_started: Notify,
        destroy_release: Notify,
        destroy_blocked: AtomicBool,
        destroy_fails: AtomicBool,
        wait_exit: Notify,
        should_exit: AtomicBool,
        opens: AtomicUsize,
        health_activity: Option<Arc<Notify>>,
    }

    #[async_trait]
    impl VmInstance for MockVm {
        fn id(&self) -> &VmId {
            &self.id
        }

        async fn start(&self) -> Result<(), VmError> {
            self.starts.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn stop(&self, _: StopMode) -> Result<(), VmError> {
            Ok(())
        }

        async fn wait(&self) -> Result<VmExit, VmError> {
            self.wait_exit.notified().await;
            if self.should_exit.load(Ordering::Relaxed) {
                Ok(VmExit {
                    code: Some(1),
                    signal: None,
                })
            } else {
                future::pending().await
            }
        }

        fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
            self.events.subscribe()
        }

        async fn open_private_service_connection(
            &self,
        ) -> Result<vm_trait::BoxedPrivateServiceConnection, VmError> {
            self.opens.fetch_add(1, Ordering::Relaxed);
            if let Some(activity) = &self.health_activity {
                activity.notify_one();
            }
            self.connections
                .lock()
                .expect("connection mutex")
                .pop_front()
                .ok_or(VmError::InvalidState("no test connection"))
        }

        async fn destroy(&self) -> Result<(), VmError> {
            self.destroy_started.notify_one();
            if self.destroy_blocked.load(Ordering::Relaxed) {
                self.destroy_release.notified().await;
            }
            self.destroys.fetch_add(1, Ordering::Relaxed);
            if self.destroy_fails.load(Ordering::Relaxed) {
                Err(VmError::Unavailable {
                    resource: String::from("test VM"),
                    reason: String::from("destroy failed"),
                })
            } else {
                Ok(())
            }
        }
    }

    struct MockOwnership {
        events: Mutex<Vec<&'static str>>,
        renewals: AtomicUsize,
        renewal_activity: Option<Arc<Notify>>,
        renew_fails: AtomicBool,
        stopping_fails: AtomicBool,
        drain_conflict: AtomicBool,
        drain_blocked: AtomicBool,
        drain_started: Notify,
        drain_release: Notify,
        promote_fails: AtomicBool,
        promote_started: Notify,
        promote_release: Notify,
        promote_blocked: AtomicBool,
    }

    struct MockFailureStore {
        reports: Mutex<Vec<(GatewayServiceInstanceLease, GatewayServiceFailure)>>,
        error: Mutex<Option<GatewayServiceFailureStoreError>>,
        started: Notify,
        release: Notify,
        blocked: AtomicBool,
    }

    #[async_trait]
    impl GatewayServiceFailureStore for MockFailureStore {
        async fn record_failure(
            &self,
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            failure: GatewayServiceFailure,
        ) -> Result<(), GatewayServiceFailureStoreError> {
            self.started.notify_one();
            if self.blocked.load(Ordering::Relaxed) {
                self.release.notified().await;
            }
            let configured_error = *self.error.lock().expect("failure error mutex");
            if let Some(error) = configured_error {
                return Err(error);
            }
            self.reports
                .lock()
                .expect("failure reports mutex")
                .push((lease.clone(), failure));
            Ok(())
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
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.renewals.fetch_add(1, Ordering::Relaxed);
            if let Some(activity) = &self.renewal_activity {
                activity.notify_one();
            }
            if self.renew_fails.load(Ordering::Relaxed) {
                Err(GatewayServiceOwnershipError::StaleLease)
            } else {
                Ok(lease.clone())
            }
        }

        async fn claim_expired(
            &self,
            _: &GatewayServiceOwner,
            _: Duration,
            _: usize,
        ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
            Ok(Vec::new())
        }

        async fn mark_stopping(
            &self,
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.events.lock().expect("events").push("stopping");
            if self.stopping_fails.load(Ordering::Relaxed) {
                return Err(GatewayServiceOwnershipError::Unavailable);
            }
            let mut lease = lease.clone();
            lease.state = GatewayServiceInstanceState::Stopping;
            Ok(lease)
        }

        async fn mark_starting(
            &self,
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.events.lock().expect("events").push("starting");
            let mut lease = lease.clone();
            lease.state = GatewayServiceInstanceState::Starting;
            Ok(lease)
        }

        async fn mark_ready(
            &self,
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.events.lock().expect("events").push("ready");
            let mut lease = lease.clone();
            lease.state = GatewayServiceInstanceState::Ready;
            Ok(lease)
        }

        async fn mark_draining(
            &self,
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.events.lock().expect("events").push("draining");
            if self.drain_conflict.load(Ordering::Relaxed) {
                return Err(GatewayServiceOwnershipError::Conflict);
            }
            self.drain_started.notify_one();
            if self.drain_blocked.load(Ordering::Relaxed) {
                self.drain_release.notified().await;
            }
            let mut lease = lease.clone();
            lease.state = GatewayServiceInstanceState::Draining;
            Ok(lease)
        }

        async fn promote_ready(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
            self.events.lock().expect("events").push("promote");
            self.promote_started.notify_one();
            if self.promote_blocked.load(Ordering::Relaxed) {
                self.promote_release.notified().await;
            }
            if self.promote_fails.load(Ordering::Relaxed) {
                return Err(GatewayServiceOwnershipError::Unavailable);
            }
            Ok(None)
        }

        async fn mark_cleaned(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<(), GatewayServiceOwnershipError> {
            self.events.lock().expect("events").push("cleaned");
            Ok(())
        }
    }

    struct MockCountState {
        responses: Mutex<VecDeque<Result<u64, GatewayEdgeError>>>,
        started: Notify,
        release: Notify,
        blocked: AtomicBool,
    }

    fn default_count_state() -> Arc<MockCountState> {
        Arc::new(MockCountState {
            responses: Mutex::new(VecDeque::new()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        })
    }

    struct MockTargets {
        target: Option<crate::GatewayServiceOwnedTarget>,
        count: Arc<MockCountState>,
    }

    #[async_trait]
    impl GatewayServiceTargetStore for MockTargets {
        async fn list_service_targets(
            &self,
            _: crate::GatewayServiceTargetPage,
        ) -> Result<crate::GatewayServiceTargetPageResult, GatewayEdgeError> {
            Ok(crate::GatewayServiceTargetPageResult {
                targets: Vec::new(),
                next_after: None,
            })
        }

        async fn get_service_target(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<Option<crate::GatewayServiceOwnedTarget>, GatewayEdgeError> {
            Ok(self.target.clone())
        }

        async fn get_service_instance(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
            Ok(None)
        }

        async fn count_accepted_service_invocations(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<u64, GatewayEdgeError> {
            Ok(0)
        }

        async fn count_accepted_service_invocations_for_instance(
            &self,
            _: GatewayServiceInstanceKey,
        ) -> Result<u64, GatewayEdgeError> {
            self.count.started.notify_one();
            if self.count.blocked.load(Ordering::Relaxed) {
                self.count.release.notified().await;
            }
            self.count
                .responses
                .lock()
                .expect("count responses")
                .pop_front()
                .unwrap_or(Ok(0))
        }

        async fn list_service_instances(
            &self,
            _: crate::GatewayServiceInstancePage,
        ) -> Result<crate::GatewayServiceInstancePageResult, GatewayEdgeError> {
            Ok(crate::GatewayServiceInstancePageResult {
                instances: Vec::new(),
                next_after: None,
            })
        }
    }

    struct BlockingTargets {
        target: Option<crate::GatewayServiceOwnedTarget>,
        started: Notify,
        release: Notify,
    }

    #[async_trait]
    impl GatewayServiceTargetStore for BlockingTargets {
        async fn list_service_targets(
            &self,
            _: crate::GatewayServiceTargetPage,
        ) -> Result<crate::GatewayServiceTargetPageResult, GatewayEdgeError> {
            Ok(crate::GatewayServiceTargetPageResult {
                targets: Vec::new(),
                next_after: None,
            })
        }

        async fn get_service_target(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<Option<crate::GatewayServiceOwnedTarget>, GatewayEdgeError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(self.target.clone())
        }

        async fn get_service_instance(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
            Ok(None)
        }

        async fn count_accepted_service_invocations(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<u64, GatewayEdgeError> {
            Ok(0)
        }

        async fn count_accepted_service_invocations_for_instance(
            &self,
            _: GatewayServiceInstanceKey,
        ) -> Result<u64, GatewayEdgeError> {
            Ok(0)
        }

        async fn list_service_instances(
            &self,
            _: crate::GatewayServiceInstancePage,
        ) -> Result<crate::GatewayServiceInstancePageResult, GatewayEdgeError> {
            Ok(crate::GatewayServiceInstancePageResult {
                instances: Vec::new(),
                next_after: None,
            })
        }
    }

    fn supervisor_policy(
        instance: ServiceInstancePolicy,
        lease: GatewayServiceLeasePolicy,
    ) -> GatewayServiceSupervisorPolicy {
        supervisor_policy_with_health(instance, lease, Duration::from_secs(10), 3)
    }

    fn supervisor_policy_with_health(
        instance: ServiceInstancePolicy,
        lease: GatewayServiceLeasePolicy,
        health_interval: Duration,
        health_failure_threshold: u32,
    ) -> GatewayServiceSupervisorPolicy {
        GatewayServiceSupervisorPolicy {
            lease,
            instance,
            health_interval,
            health_failure_threshold,
            ..GatewayServiceSupervisorPolicy::default()
        }
    }

    fn failure_store() -> Arc<MockFailureStore> {
        Arc::new(MockFailureStore {
            reports: Mutex::new(Vec::new()),
            error: Mutex::new(None),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        })
    }

    fn identity() -> GatewayServiceIdentity {
        GatewayServiceIdentity {
            instance_id: Uuid::from_u128(1),
            gateway_id: Uuid::from_u128(2),
            revision_id: Uuid::from_u128(3),
        }
    }

    fn launch(identity: GatewayServiceIdentity) -> GatewayServiceLaunch {
        let service = GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/ready").expect("readiness"),
            ServiceProbePath::parse("/health").expect("health"),
        )
        .expect("service");
        GatewayServiceLaunch {
            identity,
            service,
            spec: VmSpec {
                id: VmId(format!("gateway-service-{}", identity.instance_id)),
                root: RootFilesystem::Directory {
                    host_path: PathBuf::from("/tmp/coordinator-test-root"),
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
                private_http_service: Some(PrivateHttpServiceSpec {
                    loopback_port: 8080,
                    max_connections: 32,
                    connect_timeout: Duration::from_secs(2),
                }),
                labels: BTreeMap::new(),
            },
        }
    }

    fn restore_target(
        identity: GatewayServiceIdentity,
        desired_revision_id: Uuid,
    ) -> crate::GatewayServiceOwnedTarget {
        let service = GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/ready").expect("readiness"),
            ServiceProbePath::parse("/health").expect("health"),
        )
        .expect("service");
        crate::GatewayServiceOwnedTarget {
            gateway_id: identity.gateway_id,
            lifecycle: String::from("enabled"),
            active_revision_id: Some(identity.revision_id),
            desired_service_revision_id: Some(desired_revision_id),
            revision: crate::GatewayServiceRevisionTarget {
                revision_id: identity.revision_id,
                release_id: Some(Uuid::from_u128(4)),
                release_state: Some(String::from("published")),
                publication_eligible: true,
                service,
            },
        }
    }

    fn lease(
        owner: &GatewayServiceOwner,
        identity: GatewayServiceIdentity,
    ) -> GatewayServiceInstanceLease {
        let now = ::time::OffsetDateTime::now_utc();
        GatewayServiceInstanceLease {
            identity,
            owner_host_id: owner.host_id.clone(),
            owner_uuid: owner.owner_uuid,
            fencing_token: 1,
            state: GatewayServiceInstanceState::Provisioning,
            vm_id: format!("gateway-service-{}", identity.instance_id),
            lease_expires_at: now + ::time::Duration::seconds(30),
            heartbeat_at: now,
        }
    }

    async fn respond(mut peer: DuplexStream) {
        let mut request = [0_u8; 512];
        let _ = peer.read(&mut request).await;
        peer.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await
            .expect("probe response");
    }

    async fn respond_status(mut peer: DuplexStream, status: u16) {
        let mut request = [0_u8; 512];
        let _ = peer.read(&mut request).await;
        let response =
            format!("HTTP/1.1 {status} Test\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        peer.write_all(response.as_bytes())
            .await
            .expect("probe response");
    }

    async fn respond_status_notifying(mut peer: DuplexStream, status: u16, replied: Arc<Notify>) {
        let mut request = [0_u8; 512];
        let _ = peer.read(&mut request).await;
        let response =
            format!("HTTP/1.1 {status} Test\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        peer.write_all(response.as_bytes())
            .await
            .expect("probe response");
        replied.notify_one();
    }

    struct DrainFixture {
        control: GatewayServiceCoordinatorControl,
        task: tokio::task::JoinHandle<Result<(), GatewayServiceCoordinatorFailure>>,
        status: watch::Receiver<GatewayServiceCoordinatorStatus>,
        vm: Arc<MockVm>,
        ownership: Arc<MockOwnership>,
        targets: Arc<MockTargets>,
        registry: GatewayServiceRegistry,
        key: GatewayServiceInstanceKey,
    }

    // The fixture deliberately assembles the complete parent-owned lifecycle.
    async fn start_drain_fixture(
        count: Arc<MockCountState>,
        drain_conflict: bool,
        drain_blocked: bool,
        drain_timeout: Duration,
    ) -> DrainFixture {
        start_drain_fixture_with_logs(
            count,
            drain_conflict,
            drain_blocked,
            drain_timeout,
            false,
            None,
            Duration::from_secs(10),
        )
        .await
    }

    #[allow(clippy::too_many_lines)]
    async fn start_drain_fixture_with_logs(
        count: Arc<MockCountState>,
        drain_conflict: bool,
        drain_blocked: bool,
        drain_timeout: Duration,
        log_capture: bool,
        log_store: Option<Arc<dyn GatewayServiceLogStore>>,
        health_interval: Duration,
    ) -> DrainFixture {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-drain-host", Uuid::new_v4()).expect("owner");
        let health_activity = Arc::new(Notify::new());
        let renewal_activity = Arc::new(Notify::new());
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: Some(Arc::clone(&health_activity)),
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let mut service_launch = launch(identity);
        if log_capture {
            service_launch.service = service_launch
                .service
                .with_log_capture_mode(gateway_domain::ServiceLogCaptureMode::Application);
        }
        let resolver = Arc::new(MockResolver {
            launch: service_launch,
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: Some(Arc::clone(&renewal_activity)),
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(drain_conflict),
            drain_blocked: AtomicBool::new(drain_blocked),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let targets = Arc::new(MockTargets {
            target: None,
            count,
        });
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(10),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failure_store(),
            resolver.clone(),
            provider,
            targets.clone(),
            registry.clone(),
            "service.test",
            GatewayServiceSupervisorPolicy {
                drain_timeout,
                health_interval,
                lease: GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_millis(10),
                },
                instance: ServiceInstancePolicy::new(
                    Duration::from_secs(2),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                ..GatewayServiceSupervisorPolicy::default()
            },
        )
        .expect("coordinator");
        let coordinator = if let Some(store) = log_store {
            coordinator.with_log_writer(GatewayServiceLogWriterConfig::new(
                store,
                ServiceLogWriterPolicy::default(),
            ))
        } else {
            coordinator
        };
        let mut status = control.subscribe();
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        timeout(Duration::from_secs(2), async {
            while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
                status.changed().await.expect("coordinator status");
            }
        })
        .await
        .expect("ready");
        let key = GatewayServiceInstanceKey {
            identity,
            fencing_token: 1,
        };
        DrainFixture {
            control,
            task,
            status,
            vm,
            ownership,
            targets,
            registry,
            key,
        }
    }

    #[tokio::test]
    // Keep the fixture-owned VM and coordinator control alive until the
    // joined task has completed its durable cleanup transition.
    #[allow(clippy::significant_drop_tightening)]
    async fn lifecycle_log_writer_flushes_after_vm_teardown_before_mark_cleaned() {
        let store = Arc::new(LifecycleLogStore {
            append_started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
            responses: Mutex::new(VecDeque::new()),
            leases: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
        let fixture = start_drain_fixture_with_logs(
            default_count_state(),
            false,
            false,
            Duration::from_secs(2),
            true,
            Some(store_port),
            Duration::from_secs(10),
        )
        .await;
        let _ = fixture.vm.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"lifecycle log".to_vec(),
        });
        fixture.control.request_drain();
        timeout(Duration::from_secs(1), store.append_started.notified())
            .await
            .expect("durable append started");
        assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(
            fixture
                .ownership
                .events
                .lock()
                .expect("ownership events")
                .last()
                .copied(),
            Some("stopping")
        );
        store.blocked.store(false, Ordering::Relaxed);
        store.release.notify_waiters();
        assert!(fixture.task.await.expect("coordinator join").is_ok());
        assert!(store.calls.load(Ordering::Relaxed) >= 1);
        assert!(store.records.load(Ordering::Relaxed) >= 1);
        assert_eq!(
            fixture
                .ownership
                .events
                .lock()
                .expect("ownership events")
                .last()
                .copied(),
            Some("cleaned")
        );
    }

    #[tokio::test]
    // Keep the fixture-owned VM and coordinator control alive until the
    // joined task has completed its durable cleanup transition.
    #[allow(clippy::significant_drop_tightening)]
    async fn application_writer_flushes_while_ready_health_and_lease_renewal_continue() {
        let store = Arc::new(LifecycleLogStore {
            append_started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
            responses: Mutex::new(VecDeque::new()),
            leases: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
        let fixture = start_drain_fixture_with_logs(
            default_count_state(),
            false,
            false,
            Duration::from_secs(2),
            true,
            Some(store_port),
            Duration::from_millis(10),
        )
        .await;
        for _ in 0..32 {
            let (client, peer) = tokio::io::duplex(4096);
            fixture
                .vm
                .connections
                .lock()
                .expect("health connections")
                .push_back(Box::new(client));
            tokio::spawn(respond(peer));
        }
        let _ = fixture.vm.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"ready log".to_vec(),
        });
        timeout(Duration::from_secs(1), store.append_started.notified())
            .await
            .expect("ready append");
        assert_eq!(
            *fixture.status.borrow(),
            GatewayServiceCoordinatorStatus::Ready
        );
        assert!(store.records.load(Ordering::Relaxed) >= 1);

        store.blocked.store(true, Ordering::Relaxed);
        let _ = fixture.vm.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stderr,
            bytes: b"blocked log".to_vec(),
        });
        timeout(Duration::from_secs(2), store.append_started.notified())
            .await
            .expect("blocked append");
        let opens_at_blocked_append = fixture.vm.opens.load(Ordering::Relaxed);
        let renewals_at_blocked_append = fixture.ownership.renewals.load(Ordering::Relaxed);
        let vm = Arc::clone(&fixture.vm);
        let ownership = Arc::clone(&fixture.ownership);
        let health_activity = Arc::clone(
            fixture
                .vm
                .health_activity
                .as_ref()
                .expect("health activity signal"),
        );
        let renewal_activity = Arc::clone(
            fixture
                .ownership
                .renewal_activity
                .as_ref()
                .expect("renewal activity signal"),
        );
        timeout(Duration::from_secs(5), async move {
            tokio::join!(
                async {
                    while vm.opens.load(Ordering::Relaxed) <= opens_at_blocked_append {
                        health_activity.notified().await;
                    }
                },
                async {
                    while ownership.renewals.load(Ordering::Relaxed) <= renewals_at_blocked_append {
                        renewal_activity.notified().await;
                    }
                },
            );
        })
        .await
        .expect("health and lease activity while append is blocked");
        assert_eq!(
            *fixture.status.borrow(),
            GatewayServiceCoordinatorStatus::Ready
        );

        store.blocked.store(false, Ordering::Relaxed);
        store.release.notify_waiters();
        fixture.control.request_drain();
        assert!(fixture.task.await.expect("coordinator join").is_ok());
        assert!(store.calls.load(Ordering::Relaxed) >= 2);
        assert!(store.records.load(Ordering::Relaxed) >= 2);
    }

    #[tokio::test]
    // Keep the fixture-owned VM and coordinator control alive until the
    // joined task has completed its durable cleanup transition.
    #[allow(clippy::significant_drop_tightening)]
    async fn disabled_capture_never_calls_configured_log_store() {
        let store = Arc::new(LifecycleLogStore {
            append_started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
            responses: Mutex::new(VecDeque::new()),
            leases: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
        let fixture = start_drain_fixture_with_logs(
            default_count_state(),
            false,
            false,
            Duration::from_secs(2),
            false,
            Some(store_port),
            Duration::from_secs(10),
        )
        .await;
        let _ = fixture.vm.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"disabled log".to_vec(),
        });
        fixture.control.request_drain();
        assert!(fixture.task.await.expect("coordinator join").is_ok());
        assert_eq!(store.calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.records.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    // Keep the fixture-owned VM and coordinator control alive until the
    // joined task has completed its durable cleanup transition.
    #[allow(clippy::significant_drop_tightening)]
    async fn blocked_final_flush_delays_cleaned_but_not_vm_destroy() {
        let store = Arc::new(LifecycleLogStore {
            append_started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
            responses: Mutex::new(VecDeque::new()),
            leases: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
        let fixture = start_drain_fixture_with_logs(
            default_count_state(),
            false,
            false,
            Duration::from_secs(2),
            true,
            Some(store_port),
            Duration::from_secs(10),
        )
        .await;
        let _ = fixture.vm.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"flush deadline".to_vec(),
        });
        let flush_started = std::time::Instant::now();
        fixture.control.request_drain();
        timeout(Duration::from_secs(1), store.append_started.notified())
            .await
            .expect("blocked append");
        assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
        let mut task = fixture.task;
        assert!(
            timeout(Duration::from_millis(500), &mut task)
                .await
                .is_err()
        );
        assert_eq!(
            fixture
                .ownership
                .events
                .lock()
                .expect("ownership events")
                .last()
                .copied(),
            Some("stopping")
        );
        assert!(
            timeout(Duration::from_secs(4), &mut task)
                .await
                .expect("bounded final flush")
                .expect("coordinator join")
                .is_ok()
        );
        assert!(flush_started.elapsed() >= Duration::from_millis(1_500));
        assert_eq!(
            fixture
                .ownership
                .events
                .lock()
                .expect("ownership events")
                .last()
                .copied(),
            Some("cleaned")
        );
    }

    #[tokio::test]
    // Keep the fixture-owned VM and coordinator control alive until the
    // joined task has completed its durable cleanup transition.
    #[allow(clippy::significant_drop_tightening)]
    async fn stale_log_lease_stops_appends_without_rebinding_fence() {
        let store = Arc::new(LifecycleLogStore {
            append_started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
            responses: Mutex::new(VecDeque::from([Err(
                crate::GatewayServiceLogStoreError::StaleLease,
            )])),
            leases: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
        let fixture = start_drain_fixture_with_logs(
            default_count_state(),
            false,
            false,
            Duration::from_secs(2),
            true,
            Some(store_port),
            Duration::from_secs(10),
        )
        .await;
        let _ = fixture.vm.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"stale lease".to_vec(),
        });
        timeout(Duration::from_secs(1), store.append_started.notified())
            .await
            .expect("stale append");
        tokio::task::yield_now().await;
        for _ in 0..3 {
            let _ = fixture.vm.events.send(VmEvent::Log {
                stream: vm_trait::LogStream::Stdout,
                bytes: b"after stale".to_vec(),
            });
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(store.calls.load(Ordering::Relaxed), 1);
        assert_eq!(store.records.load(Ordering::Relaxed), 1);
        assert_eq!(
            store.leases.lock().expect("log store leases").as_slice(),
            &[(fixture.key.identity, fixture.key.fencing_token)]
        );
        fixture.control.request_drain();
        assert!(fixture.task.await.expect("coordinator join").is_ok());
    }

    fn drain_request() -> GatewayRequest {
        GatewayRequest {
            method: http::Method::GET,
            path_and_query: String::from("/drain-check"),
            headers: http::HeaderMap::new(),
            body: bytes::Bytes::new(),
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Http,
                authority: String::from("service.test"),
                client_address: "127.0.0.1".parse().expect("client address"),
                request_id: Uuid::new_v4(),
            },
        }
    }

    fn drain_http_policy() -> ServiceHttpPolicy {
        ServiceHttpPolicy::from_gateway_limits(GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 256,
            execution_timeout: Duration::from_secs(1),
        })
    }

    #[tokio::test]
    // The fixture owns control and VM resources until the coordinator task joins.
    #[allow(clippy::significant_drop_tightening)]
    async fn drain_keeps_registry_dispatch_until_accepted_count_reaches_zero() {
        let count = default_count_state();
        count.responses.lock().expect("count responses").extend([
            Err(GatewayEdgeError::Unavailable),
            Ok(1),
            Ok(0),
        ]);
        let fixture =
            start_drain_fixture(count.clone(), false, false, Duration::from_secs(2)).await;
        fixture.control.request_drain();
        let mut status = fixture.status;
        timeout(Duration::from_secs(1), async {
            while *status.borrow() != GatewayServiceCoordinatorStatus::Draining {
                status.changed().await.expect("draining status");
            }
        })
        .await
        .expect("draining");
        count.started.notified().await;
        let (client, peer) = tokio::io::duplex(4096);
        fixture
            .vm
            .connections
            .lock()
            .expect("connections")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let response = fixture
            .registry
            .exchange(fixture.key, drain_request(), drain_http_policy())
            .await
            .expect("accepted call remains dispatchable while draining");
        assert_eq!(response.status, http::StatusCode::OK);
        assert!(fixture.task.await.expect("coordinator join").is_ok());
        assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(
            fixture
                .targets
                .count
                .responses
                .lock()
                .expect("responses")
                .len(),
            0
        );
        assert_eq!(
            fixture.ownership.events.lock().expect("events").as_slice(),
            [
                "starting", "ready", "promote", "draining", "stopping", "cleaned"
            ]
        );
    }

    #[tokio::test]
    // The fixture owns control and VM resources until the coordinator task joins.
    #[allow(clippy::significant_drop_tightening)]
    async fn drain_conflict_consumes_request_without_spinning_or_leaving_ready() {
        let count = default_count_state();
        let fixture = start_drain_fixture(count.clone(), true, false, Duration::from_secs(1)).await;
        fixture.control.request_drain();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            *fixture.status.borrow(),
            GatewayServiceCoordinatorStatus::Ready
        );
        assert!(count.responses.lock().expect("count responses").is_empty());
        assert_eq!(
            fixture
                .ownership
                .events
                .lock()
                .expect("events")
                .iter()
                .filter(|event| **event == "draining")
                .count(),
            1
        );
        fixture
            .ownership
            .drain_conflict
            .store(false, Ordering::Relaxed);
        fixture.control.request_drain();
        timeout(Duration::from_secs(1), async {
            while *fixture.status.borrow() != GatewayServiceCoordinatorStatus::Stopped {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second drain retires instance");
        assert!(fixture.task.await.expect("coordinator join").is_ok());
    }

    #[tokio::test]
    // The fixture owns control and VM resources until the coordinator task joins.
    #[allow(clippy::significant_drop_tightening)]
    async fn drain_deadline_retires_without_failure_backoff() {
        let count = default_count_state();
        count.blocked.store(true, Ordering::Relaxed);
        let fixture =
            start_drain_fixture(count.clone(), false, false, Duration::from_millis(40)).await;
        fixture.control.request_drain();
        count.started.notified().await;
        assert!(fixture.task.await.expect("coordinator join").is_ok());
        assert!(
            fixture
                .targets
                .count
                .responses
                .lock()
                .expect("count responses")
                .is_empty()
        );
    }

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)]
    // The fixture retains the coordinator control and VM until its joined task
    // proves the blocked durable transition settled within the total budget.
    async fn drain_deadline_includes_mark_draining_transition() {
        let count = default_count_state();
        let fixture = start_drain_fixture(count, false, true, Duration::from_millis(40)).await;
        fixture.control.request_drain();
        fixture.ownership.drain_started.notified().await;
        assert!(
            timeout(Duration::from_secs(1), fixture.task)
                .await
                .expect("mark draining deadline")
                .expect("coordinator join")
                .is_ok()
        );
        assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    // The fixture owns control and VM resources until the coordinator task joins.
    #[allow(clippy::significant_drop_tightening)]
    async fn lease_loss_during_blocked_drain_count_still_cleans() {
        let count = default_count_state();
        count.blocked.store(true, Ordering::Relaxed);
        let fixture =
            start_drain_fixture(count.clone(), false, false, Duration::from_secs(2)).await;
        fixture.control.request_drain();
        count.started.notified().await;
        fixture.ownership.renew_fails.store(true, Ordering::Relaxed);
        let result = timeout(Duration::from_secs(1), fixture.task)
            .await
            .expect("lease loss cleanup")
            .expect("coordinator join");
        assert!(result.is_err());
        assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)]
    // The blocked count future remains owned while cancellation drives normal
    // teardown, so the VM cannot be abandoned by the parent request.
    async fn cancellation_during_blocked_drain_count_still_cleans() {
        let count = default_count_state();
        count.blocked.store(true, Ordering::Relaxed);
        let fixture = start_drain_fixture(count, false, false, Duration::from_secs(2)).await;
        fixture.control.request_drain();
        fixture.targets.count.started.notified().await;
        fixture.control.cancel();
        assert!(
            timeout(Duration::from_secs(1), fixture.task)
                .await
                .expect("cancellation cleanup")
                .expect("coordinator join")
                .is_ok()
        );
        assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
    }

    async fn respond_status_after_gate(
        mut peer: DuplexStream,
        status: u16,
        started: Arc<Notify>,
        release: Arc<Notify>,
        replied: Arc<Notify>,
    ) {
        let mut request = [0_u8; 512];
        let _ = peer.read(&mut request).await;
        started.notify_one();
        release.notified().await;
        let response =
            format!("HTTP/1.1 {status} Test\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        peer.write_all(response.as_bytes())
            .await
            .expect("probe response");
        replied.notify_one();
    }

    #[tokio::test(start_paused = true)]
    async fn startup_deadline_cancels_preparation_before_provisioning() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: None,
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let (events, _) = broadcast::channel(2);
        let _vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let failures = failure_store();
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_millis(10),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership,
            failures.clone(),
            resolver.clone(),
            provider.clone(),
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let run = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        tokio::time::advance(Duration::from_millis(20)).await;
        tokio::task::yield_now().await;
        resolver.release.notify_one();
        let failure = run.await.expect("coordinator join").expect_err("deadline");
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::StartupDeadline
        );
        assert_eq!(
            failures
                .reports
                .lock()
                .expect("failure reports")
                .last()
                .map(|(_, failure)| failure.code),
            Some(GatewayServiceFailureCode::Startup)
        );
        assert_eq!(provider.provisions.load(Ordering::Relaxed), 0);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
        control.cancel();
    }

    #[tokio::test]
    // The test keeps the full startup, dispatch, and cleanup ordering in one
    // scenario; the added drain controls push it just over Clippy's line bound.
    #[allow(clippy::too_many_lines)]
    async fn readiness_registers_promotes_and_explicit_shutdown_cleans() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let failures = failure_store();
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(2),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failures.clone(),
            resolver.clone(),
            provider.clone(),
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let mut status = control.subscribe();
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
            status.changed().await.expect("coordinator status");
        }
        control.cancel();
        assert!(task.await.expect("coordinator join").is_ok());
        assert!(failures.reports.lock().expect("failure reports").is_empty());
        assert_eq!(provider.provisions.load(Ordering::Relaxed), 1);
        assert_eq!(vm.starts.load(Ordering::Relaxed), 1);
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
        assert_eq!(
            ownership.events.lock().expect("events").as_slice(),
            ["starting", "ready", "promote", "stopping", "cleaned"]
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Keeps durable reporting beside lifecycle setup.
    async fn failed_readiness_never_registers_or_promotes() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond_status(peer, 503));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let failures = failure_store();
        *failures.error.lock().expect("failure error mutex") =
            Some(GatewayServiceFailureStoreError::Unavailable);
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(10),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failures.clone(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(2),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        let failure = task
            .await
            .expect("coordinator join")
            .expect_err("readiness");
        control.cancel();
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::Runtime
        );
        assert_eq!(
            failure.pending_failure.map(|failure| failure.code),
            Some(GatewayServiceFailureCode::Readiness)
        );
        assert!(failure.physical_cleanup_complete);
        assert!(!failure.durable_cleanup_complete);
        assert_eq!(vm.starts.load(Ordering::Relaxed), 1);
        assert_eq!(
            ownership.events.lock().expect("events").as_slice(),
            ["starting", "stopping"]
        );
    }

    #[tokio::test]
    async fn failed_promotion_unregisters_and_cleans() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(true),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(2),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failure_store(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        let failure = task
            .await
            .expect("coordinator join")
            .expect_err("promotion");
        control.cancel();
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::Ownership
        );
        assert_eq!(
            ownership.events.lock().expect("events").as_slice(),
            ["starting", "ready", "promote", "stopping", "cleaned"]
        );
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Keeps exit metadata and cleanup ordering together.
    async fn worker_exit_during_blocked_promotion_aborts_activation() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events: events.clone(),
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(true),
        });
        let failures = failure_store();
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(2),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failures.clone(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        ownership.promote_started.notified().await;
        vm.should_exit.store(true, Ordering::Relaxed);
        vm.wait_exit.notify_one();
        tokio::task::yield_now().await;
        ownership.promote_release.notify_one();
        let failure = task
            .await
            .expect("coordinator join")
            .expect_err("worker exit");
        control.cancel();
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::Runtime
        );
        assert_eq!(
            failures
                .reports
                .lock()
                .expect("failure reports")
                .last()
                .map(|(_, failure)| (failure.code, failure.exit_code, failure.exit_signal)),
            Some((GatewayServiceFailureCode::UnexpectedExit, Some(1), None))
        );
        assert_eq!(
            ownership.events.lock().expect("events").as_slice(),
            ["starting", "ready", "promote", "stopping", "cleaned"]
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Keeps retained ownership and failed reporting together.
    async fn cleanup_failure_retains_vm_and_materialization() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(true),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let original_vm: Arc<dyn VmInstance> = vm.clone();
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        ownership.stopping_fails.store(true, Ordering::Relaxed);
        let failures = failure_store();
        *failures.error.lock().expect("failure error mutex") =
            Some(GatewayServiceFailureStoreError::Unavailable);
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(2),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership,
            failures.clone(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        let mut status = control.subscribe();
        while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
            status.changed().await.expect("coordinator status");
        }
        control.cancel();
        let failure = task.await.expect("coordinator join").expect_err("cleanup");
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::Cancelled
        );
        assert!(
            failure
                .vm
                .as_ref()
                .is_some_and(|retained| Arc::ptr_eq(retained, &original_vm))
        );
        assert!(failure.materialization_owned);
        assert!(!failure.physical_cleanup_complete);
        assert!(!failure.durable_cleanup_complete);
        assert_eq!(
            failure.pending_failure.map(|failure| failure.code),
            Some(GatewayServiceFailureCode::Cleanup)
        );
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn lease_loss_during_provision_destroys_late_vm_without_start() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(true),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(30),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failure_store(),
            resolver.clone(),
            provider.clone(),
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        provider.started.notified().await;
        ownership.renew_fails.store(true, Ordering::Relaxed);
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::task::yield_now().await;
        provider.release.notify_one();
        let failure = task
            .await
            .expect("coordinator join")
            .expect_err("lease loss");
        control.cancel();
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::LeaseLost
        );
        assert_eq!(vm.starts.load(Ordering::Relaxed), 0);
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn blocked_cleanup_keeps_lease_monitor_running() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(true),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(10),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failure_store(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_millis(10),
                },
            ),
        )
        .expect("coordinator");
        let mut status = control.subscribe();
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
            status.changed().await.expect("coordinator status");
        }
        control.cancel();
        vm.destroy_started.notified().await;
        let renewals_at_destroy_start = ownership.renewals.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(ownership.renewals.load(Ordering::Relaxed) > renewals_at_destroy_start);
        vm.destroy_release.notify_one();
        assert!(task.await.expect("coordinator join").is_ok());
    }

    #[tokio::test]
    async fn restore_active_preserves_a_newer_desired_revision() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(2),
            GatewayServiceStartupIntent::RestoreActive,
            ownership.clone(),
            failure_store(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: Some(restore_target(identity, Uuid::from_u128(5))),
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let mut status = control.subscribe();
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
            status.changed().await.expect("coordinator status");
        }
        control.cancel();
        assert!(task.await.expect("coordinator join").is_ok());
        assert_eq!(
            ownership.events.lock().expect("events").as_slice(),
            ["starting", "ready", "stopping", "cleaned"]
        );
    }

    #[tokio::test(start_paused = true)]
    #[allow(clippy::too_many_lines)] // Keeps blocked restore cancellation and cleanup together.
    async fn lease_loss_during_restore_query_stops_promptly() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (client, peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        tokio::spawn(respond(peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let targets = Arc::new(BlockingTargets {
            target: Some(restore_target(identity, Uuid::from_u128(5))),
            started: Notify::new(),
            release: Notify::new(),
        });
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(30),
            GatewayServiceStartupIntent::RestoreActive,
            ownership.clone(),
            failure_store(),
            resolver.clone(),
            provider,
            targets.clone(),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy(
                ServiceInstancePolicy::new(
                    Duration::from_secs(120),
                    Duration::from_millis(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
            ),
        )
        .expect("coordinator");
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        targets.started.notified().await;
        ownership.renew_fails.store(true, Ordering::Relaxed);
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::task::yield_now().await;
        let join = tokio::time::timeout(Duration::from_secs(1), task);
        tokio::pin!(join);
        tokio::time::advance(Duration::from_secs(1)).await;
        let failure = join
            .await
            .expect("coordinator must stop while restore query remains blocked")
            .expect("coordinator join")
            .expect_err("lease loss");
        control.cancel();
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::LeaseLost
        );
        assert!(failure.vm.is_none());
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Sequence controlled health replies and lifecycle assertions.
    async fn serving_health_survives_one_failure_and_resets_after_success() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (client, peer) = tokio::io::duplex(4096);
        let (failed_health_client, failed_health_peer) = tokio::io::duplex(4096);
        let (successful_health_client, successful_health_peer) = tokio::io::duplex(4096);
        let (second_failed_health_client, second_failed_health_peer) = tokio::io::duplex(4096);
        let (fourth_failed_health_client, fourth_failed_health_peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(client));
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(failed_health_client));
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(successful_health_client));
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(second_failed_health_client));
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(fourth_failed_health_client));
        tokio::spawn(respond(peer));
        let first_reply = Arc::new(Notify::new());
        let second_reply = Arc::new(Notify::new());
        let third_reply = Arc::new(Notify::new());
        let fourth_started = Arc::new(Notify::new());
        let fourth_release = Arc::new(Notify::new());
        let fourth_reply = Arc::new(Notify::new());
        tokio::spawn(respond_status_notifying(
            failed_health_peer,
            503,
            first_reply.clone(),
        ));
        tokio::spawn(respond_status_notifying(
            successful_health_peer,
            200,
            second_reply.clone(),
        ));
        tokio::spawn(respond_status_notifying(
            second_failed_health_peer,
            503,
            third_reply.clone(),
        ));
        tokio::spawn(respond_status_after_gate(
            fourth_failed_health_peer,
            503,
            fourth_started.clone(),
            fourth_release.clone(),
            fourth_reply.clone(),
        ));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let failures = failure_store();
        failures.blocked.store(true, Ordering::Relaxed);
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(5),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failures.clone(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy_with_health(
                ServiceInstancePolicy::new(
                    Duration::from_secs(2),
                    Duration::from_millis(1),
                    Duration::from_millis(100),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_millis(10),
                },
                Duration::from_millis(10),
                2,
            ),
        )
        .expect("coordinator");
        let mut status = control.subscribe();
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
            status.changed().await.expect("coordinator status");
        }

        timeout(Duration::from_secs(1), async {
            while vm.opens.load(Ordering::Relaxed) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first health probe");
        first_reply.notified().await;
        assert!(!task.is_finished(), "one health failure is tolerated");
        timeout(Duration::from_secs(1), async {
            while vm.opens.load(Ordering::Relaxed) < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("successful health probe");
        second_reply.notified().await;
        timeout(Duration::from_secs(1), async {
            while vm.opens.load(Ordering::Relaxed) < 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("post-reset health probe");
        third_reply.notified().await;
        timeout(Duration::from_secs(1), fourth_started.notified())
            .await
            .expect("post-reset failure must be consumed before threshold");
        assert!(
            !task.is_finished(),
            "reset keeps the first post-success failure alive while next probe is blocked"
        );
        fourth_release.notify_one();
        fourth_reply.notified().await;
        timeout(Duration::from_secs(1), failures.started.notified())
            .await
            .expect("failure report starts");
        let renewals_while_reporting = ownership.renewals.load(Ordering::Relaxed);
        timeout(Duration::from_secs(1), async {
            while ownership.renewals.load(Ordering::Relaxed) <= renewals_while_reporting {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("lease monitor must continue while failure recording is blocked");
        failures.release.notify_one();

        let failure = timeout(Duration::from_secs(1), task)
            .await
            .expect("health threshold");
        let failure = failure
            .expect("coordinator join")
            .expect_err("health threshold failure");
        control.cancel();
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::Health
        );
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
        assert_eq!(
            failures
                .reports
                .lock()
                .expect("failure reports")
                .last()
                .map(|(_, failure)| failure.code),
            Some(GatewayServiceFailureCode::Health)
        );
    }

    #[tokio::test(start_paused = true)]
    #[allow(clippy::too_many_lines)] // Keeps the blocked-probe ownership barrier explicit.
    async fn lease_loss_during_blocked_health_probe_stops_promptly() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (ready_client, ready_peer) = tokio::io::duplex(4096);
        let (health_client, _health_peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(ready_client));
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(health_client));
        tokio::spawn(respond(ready_peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(30),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            failure_store(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy_with_health(
                ServiceInstancePolicy::new(
                    Duration::from_secs(2),
                    Duration::from_millis(1),
                    Duration::from_millis(100),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_millis(20),
                },
                Duration::from_millis(1),
                3,
            ),
        )
        .expect("coordinator");
        let mut status = control.subscribe();
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
            status.changed().await.expect("coordinator status");
        }
        tokio::time::advance(Duration::from_millis(1)).await;
        timeout(Duration::from_secs(1), async {
            while vm.opens.load(Ordering::Relaxed) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("blocked health probe");
        ownership.renew_fails.store(true, Ordering::Relaxed);
        tokio::time::advance(Duration::from_millis(25)).await;
        tokio::task::yield_now().await;
        let failure = timeout(Duration::from_secs(1), task)
            .await
            .expect("lease loss must cancel blocked health probe")
            .expect("coordinator join")
            .expect_err("lease loss");
        control.cancel();
        assert_eq!(
            failure.reason,
            GatewayServiceCoordinatorFailureReason::LeaseLost
        );
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    #[allow(clippy::too_many_lines)] // Keeps cancellation and cleanup ordering explicit.
    async fn cancellation_during_blocked_health_probe_cleans_promptly() {
        let identity = identity();
        let owner =
            GatewayServiceOwner::new("coordinator-test-host", Uuid::new_v4()).expect("owner");
        let (events, _) = broadcast::channel(2);
        let vm = Arc::new(MockVm {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            connections: Mutex::new(VecDeque::new()),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_started: Notify::new(),
            destroy_release: Notify::new(),
            destroy_blocked: AtomicBool::new(false),
            destroy_fails: AtomicBool::new(false),
            wait_exit: Notify::new(),
            should_exit: AtomicBool::new(false),
            opens: AtomicUsize::new(0),
            health_activity: None,
        });
        let (ready_client, ready_peer) = tokio::io::duplex(4096);
        let (health_client, _health_peer) = tokio::io::duplex(4096);
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(ready_client));
        vm.connections
            .lock()
            .expect("connection mutex")
            .push_back(Box::new(health_client));
        tokio::spawn(respond(ready_peer));
        let resolver = Arc::new(MockResolver {
            launch: launch(identity),
            started: Notify::new(),
            release: Notify::new(),
            cleanups: AtomicUsize::new(0),
        });
        let provider = Arc::new(MockProvider {
            provisions: AtomicUsize::new(0),
            vm: Some(vm.clone()),
            started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(false),
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renewal_activity: None,
            renew_fails: AtomicBool::new(false),
            stopping_fails: AtomicBool::new(false),
            drain_conflict: AtomicBool::new(false),
            drain_blocked: AtomicBool::new(false),
            drain_started: Notify::new(),
            drain_release: Notify::new(),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(false),
        });
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(30),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership,
            failure_store(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: None,
                count: default_count_state(),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
            supervisor_policy_with_health(
                ServiceInstancePolicy::new(
                    Duration::from_secs(2),
                    Duration::from_millis(1),
                    Duration::from_millis(100),
                    Duration::from_secs(1),
                ),
                GatewayServiceLeasePolicy {
                    lease_duration: Duration::from_secs(30),
                    renewal_interval: Duration::from_secs(5),
                },
                Duration::from_millis(1),
                3,
            ),
        )
        .expect("coordinator");
        let mut status = control.subscribe();
        let task = tokio::spawn(coordinator.run());
        resolver.started.notified().await;
        resolver.release.notify_one();
        while *status.borrow() != GatewayServiceCoordinatorStatus::Ready {
            status.changed().await.expect("coordinator status");
        }
        tokio::time::advance(Duration::from_millis(1)).await;
        timeout(Duration::from_secs(1), async {
            while vm.opens.load(Ordering::Relaxed) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("blocked health probe");
        control.cancel();
        let mut task = Box::pin(task);
        let joined = tokio::time::timeout(Duration::from_secs(1), &mut task);
        tokio::pin!(joined);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(
            joined
                .await
                .expect("cancellation deadline")
                .expect("join")
                .is_ok()
        );
        assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    }

    struct LifecycleLogStore {
        append_started: Notify,
        release: Notify,
        blocked: AtomicBool,
        calls: AtomicUsize,
        records: AtomicUsize,
        responses: Mutex<
            VecDeque<
                Result<crate::GatewayServiceLogAppendOutcome, crate::GatewayServiceLogStoreError>,
            >,
        >,
        leases: Mutex<Vec<(GatewayServiceIdentity, i64)>>,
        events: Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl GatewayServiceLogStore for LifecycleLogStore {
        async fn append_batch(
            &self,
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            batch: crate::GatewayServiceLogAppendBatch,
        ) -> Result<crate::GatewayServiceLogAppendOutcome, crate::GatewayServiceLogStoreError>
        {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.records
                .fetch_add(batch.records.len(), Ordering::Relaxed);
            self.leases
                .lock()
                .expect("log store leases")
                .push((lease.identity, lease.fencing_token));
            self.append_started.notify_one();
            if self.blocked.load(Ordering::Relaxed) {
                self.release.notified().await;
            }
            self.events
                .lock()
                .expect("log store events")
                .push("log-flush");
            self.responses
                .lock()
                .expect("log store responses")
                .pop_front()
                .unwrap_or_else(|| Ok(crate::GatewayServiceLogAppendOutcome::default()))
        }
    }

    #[tokio::test]
    async fn lifecycle_writer_does_not_starve_worker_and_flushes_after_teardown() {
        let identity = identity();
        let owner = GatewayServiceOwner::new("writer-test-host", Uuid::new_v4()).expect("owner");
        let lease = lease(&owner, identity);
        let store = Arc::new(LifecycleLogStore {
            append_started: Notify::new(),
            release: Notify::new(),
            blocked: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
            responses: Mutex::new(VecDeque::new()),
            leases: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
        let buffer = crate::ServiceLogBufferHandle::new();
        buffer.try_record(
            vm_trait::LogStream::Stdout,
            ::time::OffsetDateTime::UNIX_EPOCH,
            b"ready-service-log",
        );
        let writer = ServiceLogWriter::new(
            buffer,
            store_port,
            lease,
            owner,
            ServiceLogWriterPolicy::default(),
        )
        .expect("writer");
        let worker_events = Arc::clone(&store);
        let worker: WorkerFuture = Box::pin(async move {
            worker_events.append_started.notified().await;
            worker_events
                .events
                .lock()
                .expect("worker events")
                .push("destroyed");
            worker_events.blocked.store(false, Ordering::Relaxed);
            worker_events.release.notify_waiters();
            Ok(())
        });

        timeout(
            Duration::from_secs(1),
            drive_worker_with_logs(worker, writer),
        )
        .await
        .expect("worker and bounded log flush")
        .expect("worker result");
        assert!(store.calls.load(Ordering::Relaxed) >= 1);
        assert!(store.records.load(Ordering::Relaxed) >= 1);
        assert_eq!(
            store
                .events
                .lock()
                .expect("log store events")
                .first()
                .copied(),
            Some("destroyed")
        );
        assert!(
            store
                .events
                .lock()
                .expect("log store events")
                .iter()
                .all(|event| *event == "destroyed" || *event == "log-flush")
        );
    }
}

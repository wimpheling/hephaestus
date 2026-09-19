//! Parent-owned startup, readiness, and cleanup for one service instance.

use std::{
    fmt,
    future::{self, Future},
    pin::Pin,
    sync::Arc,
};

use tokio::{
    sync::watch,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;
use vm_trait::{VmInstance, VmProvider};

use crate::{
    GatewayServiceIdentity, GatewayServiceInstanceKey, GatewayServiceInstanceLease,
    GatewayServiceInstanceState, GatewayServiceLaunchRequest, GatewayServiceLaunchResolver,
    GatewayServiceLeaseControl, GatewayServiceLeaseMonitor, GatewayServiceLeasePolicy,
    GatewayServiceLeaseRunResult, GatewayServiceLeaseStatus, GatewayServiceOwner,
    GatewayServiceOwnership, GatewayServiceOwnershipError, GatewayServiceRegistry,
    GatewayServiceTargetStore, PreparedGatewayService, ServiceInstanceError, ServiceInstanceHandle,
    ServiceInstancePolicy, ServicePreparationFailure, ServiceWorkerState, new_service_instance,
    new_service_preparation,
};

type MonitorFuture = Pin<Box<dyn Future<Output = GatewayServiceLeaseRunResult> + Send>>;
type WorkerFuture = Pin<Box<dyn Future<Output = Result<(), ServiceInstanceError>> + Send>>;

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
    status: watch::Sender<GatewayServiceCoordinatorStatus>,
}

impl GatewayServiceCoordinatorControl {
    /// Requests shutdown; the run future still settles owned work.
    pub fn cancel(&self) {
        self.cancellation.cancel();
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
    /// Provider or materializer cleanup was not confirmed.
    CleanupIncomplete,
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
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    provider: Arc<dyn VmProvider>,
    targets: Arc<dyn GatewayServiceTargetStore>,
    registry: GatewayServiceRegistry,
    service_authority: String,
    worker_policy: ServiceInstancePolicy,
    startup_deadline: Instant,
    lease_control: GatewayServiceLeaseControl,
    lease_monitor: Option<GatewayServiceLeaseMonitor<dyn GatewayServiceOwnership>>,
    cancellation: CancellationToken,
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
        resolver: Arc<dyn GatewayServiceLaunchResolver>,
        provider: Arc<dyn VmProvider>,
        targets: Arc<dyn GatewayServiceTargetStore>,
        registry: GatewayServiceRegistry,
        service_authority: impl Into<String>,
        worker_policy: ServiceInstancePolicy,
        lease_policy: GatewayServiceLeasePolicy,
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
            lease_policy,
            initial_lease_deadline,
        )
        .map_err(|_| GatewayServiceCoordinatorError::InvalidLeasePolicy)?;
        let cancellation = CancellationToken::new();
        let (status, _) = watch::channel(GatewayServiceCoordinatorStatus::Preparing);
        let control = GatewayServiceCoordinatorControl {
            cancellation: cancellation.clone(),
            status: status.clone(),
        };
        Ok((
            Self {
                lease,
                owner,
                intent,
                ownership,
                resolver,
                provider,
                targets,
                registry,
                service_authority,
                worker_policy,
                startup_deadline,
                lease_control,
                lease_monitor: Some(lease_monitor),
                cancellation,
                status,
            },
            control,
        ))
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
        let result = self
            .run_inner(&mut monitor, &mut monitor_done, &mut status)
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
            remaining.min(self.worker_policy.startup_timeout),
            self.worker_policy.probe_interval,
            self.worker_policy.probe_timeout,
            self.worker_policy.shutdown_timeout,
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
        let mut worker: WorkerFuture = Box::pin(worker.run());
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

        loop {
            tokio::select! {
                result = &mut worker => { worker_result = Some(result); return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::Runtime).await; }
                () = self.cancellation.cancelled() => return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::Cancelled).await,
                signal = monitor_signal(monitor, *monitor_done) => { let _ = signal; *monitor_done = true; return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::LeaseLost).await; }
                changed = lease_status.changed() => if changed.is_err() || !lease_active(lease_status) { return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::LeaseLost).await; },
                changed = worker_state.changed() => if changed.is_err() || *worker_state.borrow() != ServiceWorkerState::Ready { return self.settle_worker(monitor, monitor_done, lease_status, &lease, &worker_handle, &mut worker, &mut worker_result, vm, Some(key), FailureReason::Runtime).await; },
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
        if matches!(result, Err(ServiceInstanceError::CleanupIncomplete)) {
            return Err(self.failure(current, reason, Some(vm), true, false, false));
        }
        let Ok(stopping) = stopping else {
            return Err(self.failure(current, reason, None, false, true, false));
        };
        let cleaned = self
            .transition(
                monitor,
                monitor_done,
                status,
                self.ownership.mark_cleaned(&stopping, &self.owner),
                false,
            )
            .await
            .is_ok();
        if !cleaned {
            return Err(self.failure(stopping, reason, None, false, true, false));
        }
        self.set_status(GatewayServiceCoordinatorStatus::Stopped);
        if reason == FailureReason::Cancelled {
            Ok(())
        } else {
            Err(self.failure(stopping, reason, None, false, true, true))
        }
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
        let Ok(stopping) = stopping else {
            return Err(self.failure(lease, reason, vm, !physical, physical, false));
        };
        let durable = physical
            && self
                .transition(
                    monitor,
                    monitor_done,
                    status,
                    self.ownership.mark_cleaned(&stopping, &self.owner),
                    false,
                )
                .await
                .is_ok();
        Err(self.failure(stopping, reason, vm, !physical, physical, durable))
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
        self.set_status(GatewayServiceCoordinatorStatus::Failed);
        GatewayServiceCoordinatorFailure {
            identity: lease.identity,
            lease,
            reason,
            vm,
            materialization_owned,
            physical_cleanup_complete: physical,
            durable_cleanup_complete: durable,
        }
    }

    fn set_status(&self, status: GatewayServiceCoordinatorStatus) {
        let _ = self.status.send(status);
    }
}

type FailureReason = GatewayServiceCoordinatorFailureReason;

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
    use crate::{GatewayEdgeError, GatewayServiceLaunch};
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
        renew_fails: AtomicBool,
        promote_fails: AtomicBool,
        promote_started: Notify,
        promote_release: Notify,
        promote_blocked: AtomicBool,
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
            Ok(lease.clone())
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

    struct MockTargets {
        target: Option<crate::GatewayServiceOwnedTarget>,
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
        });
        let ownership = Arc::new(MockOwnership {
            events: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            renew_fails: AtomicBool::new(false),
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
            now + Duration::from_millis(10),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership,
            resolver.clone(),
            provider.clone(),
            Arc::new(MockTargets { target: None }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
        assert_eq!(provider.provisions.load(Ordering::Relaxed), 0);
        assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
        control.cancel();
    }

    #[tokio::test]
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
            renew_fails: AtomicBool::new(false),
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
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            resolver.clone(),
            provider.clone(),
            Arc::new(MockTargets { target: None }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
            renew_fails: AtomicBool::new(false),
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
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets { target: None }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
            GatewayServiceCoordinatorFailureReason::StartupDeadline
        );
        assert_eq!(vm.starts.load(Ordering::Relaxed), 1);
        assert_eq!(
            ownership.events.lock().expect("events").as_slice(),
            ["starting", "stopping", "cleaned"]
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
            renew_fails: AtomicBool::new(false),
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
            resolver.clone(),
            provider,
            Arc::new(MockTargets { target: None }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
            renew_fails: AtomicBool::new(false),
            promote_fails: AtomicBool::new(false),
            promote_started: Notify::new(),
            promote_release: Notify::new(),
            promote_blocked: AtomicBool::new(true),
        });
        let now = Instant::now();
        let (coordinator, control) = GatewayServiceCoordinator::new(
            lease(&owner, identity),
            owner,
            now + Duration::from_secs(30),
            now + Duration::from_secs(2),
            GatewayServiceStartupIntent::ActivateDesired,
            ownership.clone(),
            resolver.clone(),
            provider,
            Arc::new(MockTargets { target: None }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
            ownership.events.lock().expect("events").as_slice(),
            ["starting", "ready", "promote", "stopping", "cleaned"]
        );
    }

    #[tokio::test]
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
            renew_fails: AtomicBool::new(false),
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
            GatewayServiceStartupIntent::ActivateDesired,
            ownership,
            resolver.clone(),
            provider,
            Arc::new(MockTargets { target: None }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
            renew_fails: AtomicBool::new(false),
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
            resolver.clone(),
            provider.clone(),
            Arc::new(MockTargets { target: None }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
            renew_fails: AtomicBool::new(false),
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
            resolver.clone(),
            provider,
            Arc::new(MockTargets { target: None }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
            renew_fails: AtomicBool::new(false),
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
            resolver.clone(),
            provider,
            Arc::new(MockTargets {
                target: Some(restore_target(identity, Uuid::from_u128(5))),
            }),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
            renew_fails: AtomicBool::new(false),
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
            resolver.clone(),
            provider,
            targets.clone(),
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            "service.test",
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
}

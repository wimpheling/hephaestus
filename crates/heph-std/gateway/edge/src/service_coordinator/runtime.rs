use super::runtime_helpers::drive_worker_with_logs;
use super::{
    Arc, FailureReason, GatewayServiceCoordinator, GatewayServiceCoordinatorFailure,
    GatewayServiceCoordinatorStatus, GatewayServiceLaunchRequest, GatewayServiceLeaseStatus,
    Instant, MonitorFuture, PrepExit, ServiceInstancePolicy, ServiceLogWriter, ServiceWorkerState,
    WorkerFuture, await_with_monitor, current_lease, lease_active, monitor_signal,
    new_service_instance, new_service_preparation, time, watch,
};

impl GatewayServiceCoordinator {
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
        self.serve_ready(
            monitor,
            monitor_done,
            lease_status,
            drain_requested,
            lease,
            worker_handle,
            worker,
            worker_state,
            worker_result,
            vm,
        )
        .await
    }
}

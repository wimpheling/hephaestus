use super::{
    Arc, DRAIN_POLL_INTERVAL, DrainCount, DrainOutcome, DrainTransition, FailureReason, Future,
    GatewayEdgeError, GatewayServiceCoordinator, GatewayServiceCoordinatorStatus,
    GatewayServiceInstanceKey, GatewayServiceInstanceLease, GatewayServiceLeaseStatus,
    GatewayServiceOwnershipError, Instant, MonitorFuture, ServiceInstanceError,
    ServiceInstanceHandle, ServiceWorkerState, VmInstance, WorkerFuture, current_deadline,
    current_lease, lease_active, monitor_signal, time, watch,
};

impl GatewayServiceCoordinator {
    // This state machine deliberately carries every live owner through the
    // drain transition, count query, and teardown boundary.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(super) async fn drain_and_settle(
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
}

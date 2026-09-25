use super::{
    Arc, DrainOutcome, FailureReason, GatewayServiceCoordinator, GatewayServiceCoordinatorFailure,
    GatewayServiceCoordinatorStatus, GatewayServiceInstanceKey, GatewayServiceInstanceLease,
    GatewayServiceLeaseStatus, GatewayServiceStartupIntent, Instant, MonitorFuture,
    ServiceInstanceError, ServiceInstanceHandle, ServiceWorkerState, VmInstance, WorkerFuture,
    current_lease, lease_active, monitor_signal, time, watch,
};

impl GatewayServiceCoordinator {
    // This select loop intentionally owns every serving signal and cleanup handle.
    #[allow(
        clippy::cognitive_complexity,
        clippy::too_many_arguments,
        clippy::too_many_lines
    )]
    pub(super) async fn serve_ready(
        &self,
        monitor: &mut MonitorFuture,
        monitor_done: &mut bool,
        lease_status: &mut watch::Receiver<GatewayServiceLeaseStatus>,
        drain_requested: &mut watch::Receiver<bool>,
        mut lease: GatewayServiceInstanceLease,
        worker_handle: ServiceInstanceHandle,
        mut worker: WorkerFuture,
        mut worker_state: watch::Receiver<ServiceWorkerState>,
        mut worker_result: Option<Result<(), ServiceInstanceError>>,
        vm: Arc<dyn VmInstance>,
    ) -> Result<(), GatewayServiceCoordinatorFailure> {
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
}

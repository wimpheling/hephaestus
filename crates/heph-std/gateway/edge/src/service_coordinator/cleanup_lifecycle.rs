use super::{
    Arc, FailureReason, GatewayServiceCoordinator, GatewayServiceCoordinatorFailure,
    GatewayServiceCoordinatorStatus, GatewayServiceFailure, GatewayServiceInstanceKey,
    GatewayServiceInstanceLease, GatewayServiceLeaseStatus, MonitorFuture, PreparedGatewayService,
    ServiceInstanceError, ServiceInstanceHandle, ServicePreparationFailure, VmInstance,
    WorkerFuture, await_with_monitor, cleanup_report, current_lease, failure_for_reason, watch,
};

impl GatewayServiceCoordinator {
    pub(super) async fn cleanup_prepared(
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

    pub(super) async fn cleanup_preparation_failure(
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

    pub(super) async fn cleanup_vm(
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
    pub(super) async fn settle_worker(
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
    pub(super) async fn cleanup_durable(
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
    pub(super) async fn finish_durable_cleanup(
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
}

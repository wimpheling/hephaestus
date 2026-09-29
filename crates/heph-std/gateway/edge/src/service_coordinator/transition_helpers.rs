use super::{
    Arc, FailureReason, Future, GatewayServiceCoordinator, GatewayServiceCoordinatorFailure,
    GatewayServiceCoordinatorStatus, GatewayServiceFailure, GatewayServiceFailureStoreError,
    GatewayServiceInstanceLease, GatewayServiceLeaseStatus, GatewayServiceOwnershipError,
    MonitorFuture, ServiceInstanceError, ServiceWorkerState, VmInstance, WorkerFuture,
    current_deadline, lease_active, monitor_signal, time, watch,
};

impl GatewayServiceCoordinator {
    pub(super) async fn transition<T, F>(
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

    pub(super) async fn record_failure(
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
    pub(super) async fn transition_with_worker<T, F>(
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
    pub(super) async fn transition_while_worker<T, F>(
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

    pub(super) fn failure(
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
    pub(super) fn failure_pending(
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

    pub(super) fn set_status(&self, status: GatewayServiceCoordinatorStatus) {
        let _ = self.status.send(status);
    }
}

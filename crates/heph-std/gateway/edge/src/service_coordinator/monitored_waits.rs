use super::{
    FailureReason, Future, GatewayServiceCoordinator, GatewayServiceLeaseStatus,
    GatewayServiceOwnershipError, MonitorFuture, ServiceInstanceError, ServiceInstanceHandle,
    ServiceWorkerState, WorkerFuture, lease_active, monitor_signal, time, watch,
};

impl GatewayServiceCoordinator {
    // The target query shares the same worker/lease select boundary as the
    // durable transitions, so all live cancellation state stays explicit.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn await_target(
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
    pub(super) async fn await_health(
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
}

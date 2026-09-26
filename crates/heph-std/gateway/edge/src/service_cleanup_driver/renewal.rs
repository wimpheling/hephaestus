use super::monitor::{MonitorState, exact_claim, map_ownership_error};
use super::{
    Arc, Duration, GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceLeaseMonitor, Instant,
    time,
};

impl GatewayServiceCleanupDriver {
    /// Renews an exact live claim before retrying cleanup.
    ///
    /// The deadline is captured before the renewal call.  A successful renewal
    /// is then transitioned to `Stopping` when necessary before the ordinary
    /// monitored cleanup state machine starts.  This keeps a terminal
    /// coordinator failure from reusing an expired monotonic lease.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when the exact claim cannot be renewed or
    /// transitioned under the current owner and fence.
    pub async fn renew_and_prepare_stopping(
        &self,
        lease: &mut GatewayServiceInstanceLease,
        initial_deadline: Instant,
    ) -> Result<Instant, GatewayServiceCleanupDriverError> {
        if initial_deadline <= Instant::now() {
            return Err(GatewayServiceCleanupDriverError::Stale);
        }
        let call_started = Instant::now();
        let database_deadline = call_started
            .checked_add(self.policy.database_timeout)
            .ok_or(GatewayServiceCleanupDriverError::InvalidInput)?;
        let renewal_deadline = initial_deadline.min(database_deadline);
        let renewed = time::timeout_at(
            renewal_deadline,
            self.ownership
                .renew(lease, &self.owner, self.policy.lease.lease_duration),
        )
        .await
        .map_err(|_| GatewayServiceCleanupDriverError::Unavailable)?
        .map_err(map_ownership_error)?;
        if !exact_claim(&renewed, lease)
            || renewed.owner_host_id != self.owner.host_id
            || renewed.owner_uuid != self.owner.owner_uuid
            || !renewed.state.is_live()
        {
            return Err(GatewayServiceCleanupDriverError::Stale);
        }
        let database_budget = Duration::try_from(renewed.lease_expires_at - renewed.heartbeat_at)
            .ok()
            .filter(|budget| !budget.is_zero())
            .ok_or(GatewayServiceCleanupDriverError::Stale)?;
        let call_deadline = call_started
            .checked_add(self.policy.lease.lease_duration)
            .ok_or(GatewayServiceCleanupDriverError::InvalidInput)?;
        let database_deadline = call_started
            .checked_add(database_budget)
            .ok_or(GatewayServiceCleanupDriverError::InvalidInput)?;
        let deadline = call_deadline.min(database_deadline);
        if deadline <= Instant::now() {
            return Err(GatewayServiceCleanupDriverError::Stale);
        }
        *lease = renewed;
        if lease.state == GatewayServiceInstanceState::Stopping {
            return Ok(deadline);
        }
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            Arc::clone(&self.ownership),
            lease.clone(),
            self.owner.clone(),
            self.policy.lease,
            deadline,
        )
        .map_err(|_| GatewayServiceCleanupDriverError::InvalidInput)?;
        let status = control.subscribe();
        let mut monitor_state = MonitorState {
            future: Box::pin(monitor.run()),
            control,
            status,
            database_timeout: self.policy.database_timeout,
            done: false,
            lost: None,
        };
        let operation_lease = lease.clone();
        let operation = self.ownership.mark_stopping(&operation_lease, &self.owner);
        let operation = match monitor_state.await_durable(operation, lease).await {
            Ok(Ok(operation)) => operation,
            Ok(Err(error)) => {
                monitor_state.stop().await;
                return Err(map_ownership_error(error));
            }
            Err(error) => {
                monitor_state.stop().await;
                return Err(error);
            }
        };
        monitor_state.stop().await;
        if Instant::now() >= deadline {
            return Err(GatewayServiceCleanupDriverError::Stale);
        }
        if !exact_claim(&operation, lease)
            || operation.owner_host_id != self.owner.host_id
            || operation.owner_uuid != self.owner.owner_uuid
            || operation.state != GatewayServiceInstanceState::Stopping
        {
            return Err(GatewayServiceCleanupDriverError::Stale);
        }
        *lease = operation;
        Ok(deadline)
    }
}

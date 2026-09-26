use super::{
    Duration, Future, GatewayServiceCleanupDriverError, GatewayServiceIdentity,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceLeaseControl,
    GatewayServiceLeaseLossReason, GatewayServiceLeaseRunResult, GatewayServiceLeaseStatus,
    GatewayServiceOwner, GatewayServiceOwnershipError, Instant, MonitorFuture, Pin, time, watch,
};

pub(super) const fn map_ownership_error(
    error: GatewayServiceOwnershipError,
) -> GatewayServiceCleanupDriverError {
    match error {
        GatewayServiceOwnershipError::StaleLease => GatewayServiceCleanupDriverError::Stale,
        GatewayServiceOwnershipError::Unavailable => GatewayServiceCleanupDriverError::Unavailable,
        GatewayServiceOwnershipError::Conflict | GatewayServiceOwnershipError::InvalidArgument => {
            GatewayServiceCleanupDriverError::Rejected
        }
    }
}

pub(super) struct MonitorState {
    pub(super) future: MonitorFuture,
    pub(super) control: GatewayServiceLeaseControl,
    pub(super) status: watch::Receiver<GatewayServiceLeaseStatus>,
    pub(super) database_timeout: Duration,
    pub(super) done: bool,
    pub(super) lost: Option<GatewayServiceLeaseLossReason>,
}

impl MonitorState {
    pub(super) async fn await_operation<T, F>(
        &mut self,
        operation: &mut Pin<Box<F>>,
        lease: &mut GatewayServiceInstanceLease,
        continue_after_loss: bool,
    ) -> Option<T>
    where
        F: Future<Output = T> + ?Sized,
    {
        loop {
            tokio::select! {
                result = operation.as_mut() => return Some(result),
                result = &mut self.future, if !self.done => {
                    self.done = true;
                    self.lost = match result { GatewayServiceLeaseRunResult::Lost(reason) => Some(reason), GatewayServiceLeaseRunResult::Stopped => Some(GatewayServiceLeaseLossReason::Expired) };
                    sync_lease(&self.status, lease);
                    if !continue_after_loss { return None; }
                    return Some(operation.as_mut().await);
                }
                changed = self.status.changed(), if !self.done => {
                    if changed.is_err() { self.done = true; self.lost = Some(GatewayServiceLeaseLossReason::Invalid); }
                    sync_lease(&self.status, lease);
                    if self.lost.is_some() && !continue_after_loss { return None; }
                }
            }
        }
    }

    pub(super) async fn await_durable<T, E, F>(
        &mut self,
        operation: F,
        lease: &mut GatewayServiceInstanceLease,
    ) -> Result<Result<T, E>, GatewayServiceCleanupDriverError>
    where
        F: Future<Output = Result<T, E>>,
    {
        if self.lost.is_some() {
            return Err(GatewayServiceCleanupDriverError::Stale);
        }
        let deadline = match &*self.status.borrow() {
            GatewayServiceLeaseStatus::Active { deadline, .. } => *deadline,
            GatewayServiceLeaseStatus::Lost(_) | GatewayServiceLeaseStatus::Stopped => {
                return Err(GatewayServiceCleanupDriverError::Stale);
            }
        };
        let local_deadline = Instant::now()
            .checked_add(self.database_timeout)
            .ok_or(GatewayServiceCleanupDriverError::InvalidInput)?;
        let deadline = deadline.min(local_deadline);
        let mut operation = Box::pin(time::timeout_at(deadline, operation));
        let result = self
            .await_operation(&mut operation, lease, false)
            .await
            .ok_or(GatewayServiceCleanupDriverError::Stale)?;
        result.map_or_else(|_| Err(GatewayServiceCleanupDriverError::Unavailable), Ok)
    }

    pub(super) async fn stop(&mut self) {
        if !self.done {
            self.control.stop();
            let _ = (&mut self.future).await;
            self.done = true;
        }
    }
}

pub(super) fn sync_lease(
    status: &watch::Receiver<GatewayServiceLeaseStatus>,
    lease: &mut GatewayServiceInstanceLease,
) {
    if let GatewayServiceLeaseStatus::Active { lease: current, .. } = &*status.borrow() {
        *lease = current.clone();
    }
}

pub(super) fn exact_claim(
    current: &GatewayServiceInstanceLease,
    expected: &GatewayServiceInstanceLease,
) -> bool {
    current.identity == expected.identity
        && current.owner_host_id == expected.owner_host_id
        && current.owner_uuid == expected.owner_uuid
        && current.fencing_token == expected.fencing_token
}

pub(super) fn validate_stopping(
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    identity: GatewayServiceIdentity,
) -> Result<(), GatewayServiceCleanupDriverError> {
    if lease.identity != identity
        || lease.state != GatewayServiceInstanceState::Stopping
        || lease.owner_host_id != owner.host_id
        || lease.owner_uuid != owner.owner_uuid
        || lease.fencing_token <= 0
        || lease.identity.instance_id.is_nil()
        || lease.identity.gateway_id.is_nil()
        || lease.identity.revision_id.is_nil()
        || lease.vm_id != format!("gateway-service-{}", lease.identity.instance_id)
    {
        return Err(GatewayServiceCleanupDriverError::InvalidInput);
    }
    Ok(())
}

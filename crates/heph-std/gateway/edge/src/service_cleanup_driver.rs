//! Fenced orchestration of physical and durable service cleanup.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio::{
    sync::watch,
    time::{self, Instant},
};
use vm_trait::VmProvider;

use crate::{
    GatewayServiceCleanup, GatewayServiceFailure, GatewayServiceFailureCode,
    GatewayServiceFailureStore, GatewayServiceFailureStoreError, GatewayServiceIdentity,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceLaunchResolver,
    GatewayServiceLeaseControl, GatewayServiceLeaseLossReason, GatewayServiceLeaseMonitor,
    GatewayServiceLeasePolicy, GatewayServiceLeaseRunResult, GatewayServiceLeaseStatus,
    GatewayServiceOwner, GatewayServiceOwnership, GatewayServiceOwnershipError,
    GatewayServiceTargetStore,
};

type MonitorFuture = Pin<Box<dyn Future<Output = GatewayServiceLeaseRunResult> + Send>>;

/// Maximum time spent on one read-only confirmation after a lease is lost.
pub const MAX_SERVICE_CLEANUP_DATABASE_WAIT: Duration = Duration::from_secs(30);

/// Policy for fenced cleanup orchestration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceCleanupDriverPolicy {
    /// Lease renewal and local deadline policy.
    pub lease: GatewayServiceLeasePolicy,
    /// Bound for one durable cleanup or confirmation operation.
    pub database_timeout: Duration,
}

impl GatewayServiceCleanupDriverPolicy {
    /// Validates cleanup timing bounds.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupDriverError::InvalidInput`] when either
    /// the lease or database timeout policy is outside its bounds.
    pub fn validate(self) -> Result<(), GatewayServiceCleanupDriverError> {
        self.lease
            .validate()
            .map_err(|_| GatewayServiceCleanupDriverError::InvalidInput)?;
        if self.database_timeout.is_zero()
            || self.database_timeout > MAX_SERVICE_CLEANUP_DATABASE_WAIT
        {
            return Err(GatewayServiceCleanupDriverError::InvalidInput);
        }
        Ok(())
    }
}

/// Redacted errors from the durable cleanup boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceCleanupDriverError {
    /// The driver, lease, or policy was malformed.
    #[error("invalid gateway service cleanup driver input")]
    InvalidInput,
    /// The exact owner or fence is no longer current; no stale write occurred.
    #[error("gateway service cleanup owner is stale")]
    Stale,
    /// Durable storage was unavailable; all caller-owned progress is retained.
    #[error("gateway service cleanup durable storage is unavailable")]
    Unavailable,
    /// The physical or durable operation was attempted after malformed input.
    #[error("gateway service cleanup durable operation was rejected")]
    Rejected,
}

/// Result of one cleanup attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceCleanupDriverOutcome {
    /// Physical and durable cleanup are confirmed.
    Cleaned,
    /// The caller retains state and must retry later.
    Pending {
        /// Whether provider teardown and materializer cleanup are complete.
        physical_complete: bool,
        /// Whether the pending failure has been durably recorded.
        failure_recorded: bool,
        /// Whether lease monitoring lost the current claim during this attempt.
        lease_lost: bool,
    },
}

/// Caller-owned fenced cleanup driver.
pub struct GatewayServiceCleanupDriver {
    ownership: Arc<dyn GatewayServiceOwnership>,
    failure_store: Arc<dyn GatewayServiceFailureStore>,
    targets: Arc<dyn GatewayServiceTargetStore>,
    provider: Arc<dyn VmProvider>,
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    owner: GatewayServiceOwner,
    policy: GatewayServiceCleanupDriverPolicy,
}

impl GatewayServiceCleanupDriver {
    /// Creates a driver with immutable dependency and owner snapshots.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupDriverError::InvalidInput`] for an
    /// invalid owner or policy.
    pub fn new(
        ownership: Arc<dyn GatewayServiceOwnership>,
        failure_store: Arc<dyn GatewayServiceFailureStore>,
        targets: Arc<dyn GatewayServiceTargetStore>,
        provider: Arc<dyn VmProvider>,
        resolver: Arc<dyn GatewayServiceLaunchResolver>,
        owner: GatewayServiceOwner,
        policy: GatewayServiceCleanupDriverPolicy,
    ) -> Result<Self, GatewayServiceCleanupDriverError> {
        owner
            .validate()
            .map_err(|_| GatewayServiceCleanupDriverError::InvalidInput)?;
        policy.validate()?;
        Ok(Self {
            ownership,
            failure_store,
            targets,
            provider,
            resolver,
            owner,
            policy,
        })
    }

    /// Performs one bounded cleanup attempt for an already-owned stopping row.
    ///
    /// The caller retains `cleanup`, `lease`, and `pending_failure` across every
    /// return.  A lease loss never abandons the physical future, and no task is
    /// detached from this call.
    ///
    /// # Errors
    ///
    /// Returns a redacted durable-boundary error while preserving all caller
    /// owned cleanup and failure state for a later attempt.
    #[allow(clippy::too_many_lines)] // One ordered state machine protects cleanup/failure/lease invariants.
    pub async fn attempt(
        &self,
        cleanup: &mut GatewayServiceCleanup,
        lease: &mut GatewayServiceInstanceLease,
        pending_failure: &mut Option<GatewayServiceFailure>,
        initial_deadline: Instant,
    ) -> Result<GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverError> {
        validate_stopping(lease, &self.owner, cleanup.identity())?;
        let identity = cleanup.identity();
        if cleanup.vm_teardown_confirmed()
            && cleanup.materializer_cleanup_confirmed()
            && initial_deadline <= Instant::now()
        {
            match self.lookup_instance(identity).await? {
                Some(current) if current.state == GatewayServiceInstanceState::Cleaned => {
                    if exact_claim(&current, lease) {
                        return Ok(GatewayServiceCleanupDriverOutcome::Cleaned);
                    }
                    return Err(GatewayServiceCleanupDriverError::Stale);
                }
                Some(_) => return Err(GatewayServiceCleanupDriverError::Stale),
                None => return Err(GatewayServiceCleanupDriverError::Unavailable),
            }
        }
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            Arc::clone(&self.ownership),
            lease.clone(),
            self.owner.clone(),
            self.policy.lease,
            initial_deadline,
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

        let current = match monitor_state
            .await_durable(self.targets.get_service_instance(identity), lease)
            .await
        {
            Ok(Ok(current)) => current,
            Ok(Err(_)) | Err(GatewayServiceCleanupDriverError::Unavailable) => {
                monitor_state.stop().await;
                return Err(GatewayServiceCleanupDriverError::Unavailable);
            }
            Err(error) => {
                monitor_state.stop().await;
                return Err(error);
            }
        };
        match current {
            Some(current)
                if exact_claim(&current, lease)
                    && current.state == GatewayServiceInstanceState::Stopping => {}
            Some(current) if current.state == GatewayServiceInstanceState::Cleaned => {
                monitor_state.stop().await;
                if cleanup.vm_teardown_confirmed()
                    && cleanup.materializer_cleanup_confirmed()
                    && exact_claim(&current, lease)
                {
                    return Ok(GatewayServiceCleanupDriverOutcome::Cleaned);
                }
                return Err(GatewayServiceCleanupDriverError::Stale);
            }
            Some(_) => {
                monitor_state.stop().await;
                return Err(GatewayServiceCleanupDriverError::Stale);
            }
            None => {
                monitor_state.stop().await;
                return Err(GatewayServiceCleanupDriverError::Unavailable);
            }
        }

        let mut physical =
            Box::pin(cleanup.attempt(self.provider.as_ref(), self.resolver.as_ref()));
        let physical_result = monitor_state
            .await_operation(&mut physical, lease, true)
            .await
            .ok_or(GatewayServiceCleanupDriverError::Unavailable)?;
        drop(physical);
        let physical_complete = physical_result.is_ok();
        if !physical_complete && pending_failure.is_none() {
            *pending_failure = Some(
                GatewayServiceFailure::new(GatewayServiceFailureCode::Cleanup, None, None)
                    .map_err(|_| GatewayServiceCleanupDriverError::Rejected)?,
            );
        }
        if let Some(reason) = monitor_state.lost {
            monitor_state.stop().await;
            let _ = reason;
            return Ok(GatewayServiceCleanupDriverOutcome::Pending {
                physical_complete,
                failure_recorded: false,
                lease_lost: true,
            });
        }

        if let Some(failure) = *pending_failure {
            let operation_lease = lease.clone();
            let operation =
                self.failure_store
                    .record_failure(&operation_lease, &self.owner, failure);
            match monitor_state.await_durable(operation, lease).await {
                Ok(Ok(())) => *pending_failure = None,
                Ok(Err(GatewayServiceFailureStoreError::StaleLease))
                | Err(GatewayServiceCleanupDriverError::Stale) => {
                    monitor_state.stop().await;
                    return Err(GatewayServiceCleanupDriverError::Stale);
                }
                Ok(Err(GatewayServiceFailureStoreError::Unavailable))
                | Err(GatewayServiceCleanupDriverError::Unavailable) => {
                    monitor_state.stop().await;
                    return Err(GatewayServiceCleanupDriverError::Unavailable);
                }
                Ok(Err(GatewayServiceFailureStoreError::InvalidArgument))
                | Err(
                    GatewayServiceCleanupDriverError::Rejected
                    | GatewayServiceCleanupDriverError::InvalidInput,
                ) => {
                    monitor_state.stop().await;
                    return Err(GatewayServiceCleanupDriverError::Rejected);
                }
            }
        }
        if !physical_complete {
            let lease_lost = monitor_state.lost.is_some();
            monitor_state.stop().await;
            return Ok(GatewayServiceCleanupDriverOutcome::Pending {
                physical_complete: false,
                failure_recorded: pending_failure.is_none(),
                lease_lost,
            });
        }
        if monitor_state.lost.is_some() {
            monitor_state.stop().await;
            return Ok(GatewayServiceCleanupDriverOutcome::Pending {
                physical_complete: true,
                failure_recorded: pending_failure.is_none(),
                lease_lost: true,
            });
        }

        let operation_lease = lease.clone();
        let operation = self.ownership.mark_cleaned(&operation_lease, &self.owner);
        match monitor_state.await_durable(operation, lease).await {
            Ok(Ok(())) => {
                monitor_state.stop().await;
                Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
            }
            Ok(Err(
                GatewayServiceOwnershipError::StaleLease
                | GatewayServiceOwnershipError::Unavailable,
            ))
            | Err(
                GatewayServiceCleanupDriverError::Stale
                | GatewayServiceCleanupDriverError::Unavailable,
            ) => {
                monitor_state.stop().await;
                if self.confirm_cleaned(identity, lease).await? {
                    Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
                } else {
                    Err(GatewayServiceCleanupDriverError::Unavailable)
                }
            }
            Ok(Err(
                GatewayServiceOwnershipError::InvalidArgument
                | GatewayServiceOwnershipError::Conflict,
            ))
            | Err(
                GatewayServiceCleanupDriverError::Rejected
                | GatewayServiceCleanupDriverError::InvalidInput,
            ) => {
                monitor_state.stop().await;
                Err(GatewayServiceCleanupDriverError::Rejected)
            }
        }
    }

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

    /// Confirms a physically complete cleanup that may already be durable.
    ///
    /// This read-only check lets a retry resolve an acknowledged `Cleaned`
    /// response without requiring a renewed lease or repeating physical work.
    /// `Ok(false)` means the exact row still needs the ordinary retry path.
    ///
    /// # Errors
    ///
    /// Returns a redacted error for an unavailable lookup or a newer owner or
    /// fence.  The caller may retry an unavailable lookup without mutation.
    pub async fn confirm_cleaned_state(
        &self,
        cleanup: &GatewayServiceCleanup,
        lease: &GatewayServiceInstanceLease,
    ) -> Result<bool, GatewayServiceCleanupDriverError> {
        if !cleanup.vm_teardown_confirmed() || !cleanup.materializer_cleanup_confirmed() {
            return Ok(false);
        }
        match self.lookup_instance(cleanup.identity()).await? {
            Some(current) if current.state == GatewayServiceInstanceState::Cleaned => {
                exact_claim(&current, lease)
                    .then_some(true)
                    .ok_or(GatewayServiceCleanupDriverError::Stale)
            }
            Some(current) if exact_claim(&current, lease) => Ok(false),
            Some(_) => Err(GatewayServiceCleanupDriverError::Stale),
            None => Err(GatewayServiceCleanupDriverError::Unavailable),
        }
    }

    async fn lookup_instance(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceCleanupDriverError> {
        time::timeout(
            self.policy.database_timeout,
            self.targets.get_service_instance(identity),
        )
        .await
        .map_err(|_| GatewayServiceCleanupDriverError::Unavailable)?
        .map_err(|_| GatewayServiceCleanupDriverError::Unavailable)
    }

    async fn confirm_cleaned(
        &self,
        identity: GatewayServiceIdentity,
        expected: &GatewayServiceInstanceLease,
    ) -> Result<bool, GatewayServiceCleanupDriverError> {
        match self.lookup_instance(identity).await? {
            Some(instance)
                if instance.identity == identity
                    && instance.state == GatewayServiceInstanceState::Cleaned =>
            {
                if exact_claim(&instance, expected) {
                    Ok(true)
                } else {
                    Err(GatewayServiceCleanupDriverError::Stale)
                }
            }
            Some(instance)
                if instance.identity == identity
                    && instance.owner_host_id == expected.owner_host_id
                    && instance.owner_uuid == expected.owner_uuid
                    && instance.fencing_token == expected.fencing_token =>
            {
                Err(GatewayServiceCleanupDriverError::Unavailable)
            }
            Some(instance) if instance.identity == identity => {
                Err(GatewayServiceCleanupDriverError::Stale)
            }
            Some(_) => Err(GatewayServiceCleanupDriverError::Stale),
            None => Err(GatewayServiceCleanupDriverError::Unavailable),
        }
    }
}

const fn map_ownership_error(
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

struct MonitorState {
    future: MonitorFuture,
    control: GatewayServiceLeaseControl,
    status: watch::Receiver<GatewayServiceLeaseStatus>,
    database_timeout: Duration,
    done: bool,
    lost: Option<GatewayServiceLeaseLossReason>,
}

impl MonitorState {
    async fn await_operation<T, F>(
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

    async fn await_durable<T, E, F>(
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

    async fn stop(&mut self) {
        if !self.done {
            self.control.stop();
            let _ = (&mut self.future).await;
            self.done = true;
        }
    }
}

fn sync_lease(
    status: &watch::Receiver<GatewayServiceLeaseStatus>,
    lease: &mut GatewayServiceInstanceLease,
) {
    if let GatewayServiceLeaseStatus::Active { lease: current, .. } = &*status.borrow() {
        *lease = current.clone();
    }
}

fn exact_claim(
    current: &GatewayServiceInstanceLease,
    expected: &GatewayServiceInstanceLease,
) -> bool {
    current.identity == expected.identity
        && current.owner_host_id == expected.owner_host_id
        && current.owner_uuid == expected.owner_uuid
        && current.fencing_token == expected.fencing_token
}

fn validate_stopping(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GatewayEdgeError, GatewayServiceInstanceKey, GatewayServiceInstancePage,
        GatewayServiceInstancePageResult, GatewayServiceLaunch, GatewayServiceLaunchRequest,
        GatewayServiceOwnedTarget, GatewayServiceTargetPage, GatewayServiceTargetPageResult,
    };
    use ::time::OffsetDateTime;
    use async_trait::async_trait;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;
    use uuid::Uuid;
    use vm_trait::{VmError, VmId, VmInstance, VmSpec};

    struct TestProvider {
        gate: Option<Arc<Notify>>,
        started: Option<Arc<Notify>>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl VmProvider for TestProvider {
        fn name(&self) -> &'static str {
            "cleanup-driver-test"
        }

        async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
            Err(VmError::Destroyed)
        }

        async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if let Some(started) = &self.started {
                started.notify_one();
            }
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            Ok(())
        }
    }

    struct TestResolver;

    #[async_trait]
    impl GatewayServiceLaunchResolver for TestResolver {
        async fn resolve_service_launch(
            &self,
            _: GatewayServiceLaunchRequest,
        ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
            Err(GatewayEdgeError::Unavailable)
        }

        async fn cleanup_service_launch(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<(), GatewayEdgeError> {
            Ok(())
        }
    }

    struct TestOwnership {
        renewals: AtomicUsize,
        mark_cleaned: AtomicUsize,
        renewal_lease: Mutex<GatewayServiceInstanceLease>,
        mark_result: Mutex<Result<(), GatewayServiceOwnershipError>>,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl GatewayServiceOwnership for TestOwnership {
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
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.renewals.fetch_add(1, Ordering::Relaxed);
            Ok(self.renewal_lease.lock().expect("renewal lease").clone())
        }
        async fn claim_expired(
            &self,
            _: &GatewayServiceOwner,
            _: Duration,
            _: usize,
        ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
            Err(GatewayServiceOwnershipError::Unavailable)
        }
        async fn mark_stopping(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Err(GatewayServiceOwnershipError::Unavailable)
        }
        async fn mark_starting(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Err(GatewayServiceOwnershipError::Unavailable)
        }
        async fn mark_ready(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Err(GatewayServiceOwnershipError::Unavailable)
        }
        async fn mark_draining(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Err(GatewayServiceOwnershipError::Unavailable)
        }
        async fn promote_ready(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
            Err(GatewayServiceOwnershipError::Unavailable)
        }
        async fn mark_cleaned(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<(), GatewayServiceOwnershipError> {
            self.mark_cleaned.fetch_add(1, Ordering::Relaxed);
            self.events.lock().expect("events").push("mark_cleaned");
            *self.mark_result.lock().expect("mark result")
        }
    }

    struct TestFailures {
        calls: AtomicUsize,
        result: Mutex<Result<(), GatewayServiceFailureStoreError>>,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl GatewayServiceFailureStore for TestFailures {
        async fn record_failure(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: GatewayServiceFailure,
        ) -> Result<(), GatewayServiceFailureStoreError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.events.lock().expect("events").push("record_failure");
            *self.result.lock().expect("failure result")
        }
    }

    struct TestTargets {
        instance: Mutex<Option<GatewayServiceInstanceLease>>,
        responses: Mutex<Vec<Option<GatewayServiceInstanceLease>>>,
    }

    #[async_trait]
    impl GatewayServiceTargetStore for TestTargets {
        async fn list_service_targets(
            &self,
            _: GatewayServiceTargetPage,
        ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
            Err(GatewayEdgeError::Unavailable)
        }
        async fn get_service_target(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<Option<GatewayServiceOwnedTarget>, GatewayEdgeError> {
            Err(GatewayEdgeError::Unavailable)
        }
        async fn count_accepted_service_invocations(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<u64, GatewayEdgeError> {
            Err(GatewayEdgeError::Unavailable)
        }
        async fn count_accepted_service_invocations_for_instance(
            &self,
            _: GatewayServiceInstanceKey,
        ) -> Result<u64, GatewayEdgeError> {
            Err(GatewayEdgeError::Unavailable)
        }
        async fn get_service_instance(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
            let response = self.responses.lock().expect("target responses").pop();
            if let Some(response) = response {
                return Ok(response);
            }
            Ok(self.instance.lock().expect("target instance").clone())
        }
        async fn list_service_instances(
            &self,
            _: GatewayServiceInstancePage,
        ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
            Err(GatewayEdgeError::Unavailable)
        }
    }

    fn fixture() -> (
        GatewayServiceOwner,
        GatewayServiceInstanceLease,
        GatewayServiceCleanupDriverPolicy,
    ) {
        let owner = GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner");
        let identity = GatewayServiceIdentity {
            instance_id: Uuid::new_v4(),
            gateway_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        };
        let now = OffsetDateTime::now_utc();
        let lease = GatewayServiceInstanceLease {
            identity,
            owner_host_id: owner.host_id.clone(),
            owner_uuid: owner.owner_uuid,
            fencing_token: 1,
            state: GatewayServiceInstanceState::Stopping,
            vm_id: format!("gateway-service-{}", identity.instance_id),
            lease_expires_at: now + ::time::Duration::seconds(10),
            heartbeat_at: now,
        };
        let policy = GatewayServiceCleanupDriverPolicy {
            lease: GatewayServiceLeasePolicy {
                lease_duration: Duration::from_millis(300),
                renewal_interval: Duration::from_millis(30),
            },
            database_timeout: Duration::from_secs(1),
        };
        (owner, lease, policy)
    }

    fn driver_parts(
        owner: &GatewayServiceOwner,
        policy: GatewayServiceCleanupDriverPolicy,
        provider: Arc<TestProvider>,
        ownership: Arc<TestOwnership>,
        failures: Arc<TestFailures>,
        targets: Arc<TestTargets>,
    ) -> GatewayServiceCleanupDriver {
        GatewayServiceCleanupDriver::new(
            ownership,
            failures,
            targets,
            provider,
            Arc::new(TestResolver),
            owner.clone(),
            policy,
        )
        .expect("driver")
    }

    #[tokio::test]
    async fn stale_retry_renewal_does_not_touch_physical_or_durable_state() {
        let (owner, lease, policy) = fixture();
        let provider = Arc::new(TestProvider {
            gate: None,
            started: None,
            calls: AtomicUsize::new(0),
        });
        let mut stale = lease.clone();
        stale.fencing_token += 1;
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(stale),
            mark_result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(lease.clone())),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            Arc::clone(&provider),
            Arc::clone(&ownership),
            Arc::clone(&failures),
            targets,
        );
        let mut lease = lease;
        let error = driver
            .renew_and_prepare_stopping(&mut lease, Instant::now() + Duration::from_secs(1))
            .await
            .expect_err("stale renewal");
        assert_eq!(error, GatewayServiceCleanupDriverError::Stale);
        assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
        assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
        assert_eq!(failures.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn confirmed_cleanup_accepts_exact_cleaned_row_without_renewal() {
        let (owner, lease, policy) = fixture();
        let provider = Arc::new(TestProvider {
            gate: None,
            started: None,
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let mut cleaned = lease.clone();
        cleaned.state = GatewayServiceInstanceState::Cleaned;
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(cleaned)),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            Arc::clone(&provider),
            Arc::clone(&ownership),
            Arc::clone(&failures),
            targets,
        );
        let cleanup =
            GatewayServiceCleanup::from_confirmed_physical(lease.identity, Duration::from_secs(1))
                .expect("confirmed cleanup");
        assert!(
            driver
                .confirm_cleaned_state(&cleanup, &lease)
                .await
                .expect("cleaned confirmation")
        );
        assert_eq!(ownership.renewals.load(Ordering::Relaxed), 0);
        assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn heartbeat_continues_while_physical_cleanup_is_blocked() {
        let (owner, lease, policy) = fixture();
        let gate = Arc::new(Notify::new());
        let started = Arc::new(Notify::new());
        let provider = Arc::new(TestProvider {
            gate: Some(Arc::clone(&gate)),
            started: Some(Arc::clone(&started)),
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(lease.clone())),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            Arc::clone(&provider),
            ownership.clone(),
            failures,
            targets,
        );
        let mut cleanup = GatewayServiceCleanup::new(lease.identity, None, Duration::from_secs(1))
            .expect("cleanup");
        let mut lease = lease;
        let mut pending = None;
        let task = tokio::spawn(async move {
            driver
                .attempt(
                    &mut cleanup,
                    &mut lease,
                    &mut pending,
                    Instant::now() + Duration::from_secs(2),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("physical cleanup started");
        let renewals_before_blocked_cleanup = ownership.renewals.load(Ordering::Relaxed);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if ownership.renewals.load(Ordering::Relaxed) > renewals_before_blocked_cleanup {
                    break;
                }
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("heartbeat during blocked physical cleanup");
        gate.notify_one();
        assert_eq!(
            task.await.expect("driver task").expect("driver result"),
            GatewayServiceCleanupDriverOutcome::Cleaned
        );
    }

    #[tokio::test]
    async fn unavailable_failure_report_preserves_pending_and_blocks_cleaned() {
        let (owner, lease, policy) = fixture();
        let provider = Arc::new(TestProvider {
            gate: None,
            started: None,
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Err(GatewayServiceFailureStoreError::Unavailable)),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(lease.clone())),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            provider,
            ownership.clone(),
            failures,
            targets,
        );
        let mut cleanup = GatewayServiceCleanup::new(lease.identity, None, Duration::from_secs(1))
            .expect("cleanup");
        let mut lease = lease;
        let failure = GatewayServiceFailure::new(GatewayServiceFailureCode::Cleanup, None, None)
            .expect("failure");
        let mut pending = Some(failure);
        assert_eq!(
            driver
                .attempt(
                    &mut cleanup,
                    &mut lease,
                    &mut pending,
                    Instant::now() + Duration::from_secs(2)
                )
                .await,
            Err(GatewayServiceCleanupDriverError::Unavailable)
        );
        assert_eq!(pending, Some(failure));
        assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn successful_failure_report_precedes_cleaned_transition() {
        let (owner, lease, policy) = fixture();
        let events = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(TestProvider {
            gate: None,
            started: None,
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Ok(())),
            events: Arc::clone(&events),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::clone(&events),
        });
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(lease.clone())),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            provider,
            ownership.clone(),
            failures.clone(),
            targets,
        );
        let mut cleanup = GatewayServiceCleanup::new(lease.identity, None, Duration::from_secs(1))
            .expect("cleanup");
        let mut lease = lease;
        let mut pending = Some(
            GatewayServiceFailure::new(GatewayServiceFailureCode::Readiness, None, None)
                .expect("failure"),
        );
        assert_eq!(
            driver
                .attempt(
                    &mut cleanup,
                    &mut lease,
                    &mut pending,
                    Instant::now() + Duration::from_secs(2)
                )
                .await,
            Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
        );
        assert!(pending.is_none());
        assert_eq!(failures.calls.load(Ordering::Relaxed), 1);
        assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 1);
        assert_eq!(
            *events.lock().expect("events"),
            vec!["record_failure", "mark_cleaned"]
        );
    }

    #[tokio::test]
    async fn ambiguous_cleaned_response_is_confirmed_without_repeating_physical_cleanup() {
        let (owner, lease, policy) = fixture();
        let provider = Arc::new(TestProvider {
            gate: None,
            started: None,
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Err(GatewayServiceOwnershipError::Unavailable)),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let mut cleaned = lease.clone();
        cleaned.state = GatewayServiceInstanceState::Cleaned;
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(cleaned.clone())),
            responses: Mutex::new(vec![Some(cleaned), Some(lease.clone())]),
        });
        let driver = driver_parts(
            &owner,
            policy,
            Arc::clone(&provider),
            ownership.clone(),
            failures,
            targets,
        );
        let mut cleanup =
            GatewayServiceCleanup::from_confirmed_physical(lease.identity, Duration::from_secs(1))
                .expect("cleanup");
        let mut lease = lease;
        let mut pending = None;
        assert_eq!(
            driver
                .attempt(
                    &mut cleanup,
                    &mut lease,
                    &mut pending,
                    Instant::now() + Duration::from_secs(2)
                )
                .await,
            Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
        );
        assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 1);
        assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn expired_deadline_confirms_cleaned_physical_state_without_renewal() {
        let (owner, lease, policy) = fixture();
        let provider = Arc::new(TestProvider {
            gate: None,
            started: None,
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let mut cleaned = lease.clone();
        cleaned.state = GatewayServiceInstanceState::Cleaned;
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(cleaned)),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            provider,
            ownership.clone(),
            failures,
            targets,
        );
        let mut cleanup =
            GatewayServiceCleanup::from_confirmed_physical(lease.identity, Duration::from_secs(1))
                .expect("cleanup");
        let mut lease = lease;
        let mut pending = None;
        assert_eq!(
            driver
                .attempt(
                    &mut cleanup,
                    &mut lease,
                    &mut pending,
                    Instant::now() - Duration::from_millis(1)
                )
                .await,
            Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
        );
        assert_eq!(ownership.renewals.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn lease_loss_still_settles_physical_cleanup_and_retains_state() {
        let (owner, lease, mut policy) = fixture();
        policy.lease.renewal_interval = Duration::from_millis(100);
        let gate = Arc::new(Notify::new());
        let started = Arc::new(Notify::new());
        let provider = Arc::new(TestProvider {
            gate: Some(Arc::clone(&gate)),
            started: Some(Arc::clone(&started)),
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(lease.clone())),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            provider,
            ownership.clone(),
            failures,
            targets,
        );
        let mut cleanup = GatewayServiceCleanup::new(lease.identity, None, Duration::from_secs(1))
            .expect("cleanup");
        let mut lease = lease;
        let mut pending = None;
        let task = tokio::spawn(async move {
            driver
                .attempt(
                    &mut cleanup,
                    &mut lease,
                    &mut pending,
                    Instant::now() + Duration::from_millis(50),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("physical cleanup started");
        tokio::time::sleep(Duration::from_millis(100)).await;
        gate.notify_one();
        assert_eq!(
            task.await.expect("driver task").expect("driver result"),
            GatewayServiceCleanupDriverOutcome::Pending {
                physical_complete: true,
                failure_recorded: false,
                lease_lost: true
            }
        );
        assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn newer_fence_does_not_allow_stale_cleanup_write() {
        let (owner, lease, policy) = fixture();
        let provider = Arc::new(TestProvider {
            gate: None,
            started: None,
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Err(GatewayServiceOwnershipError::StaleLease)),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let mut newer = lease.clone();
        newer.fencing_token += 1;
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(newer)),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            provider,
            ownership.clone(),
            failures,
            targets,
        );
        let mut cleanup = GatewayServiceCleanup::new(lease.identity, None, Duration::from_secs(1))
            .expect("cleanup");
        let mut lease = lease;
        let mut pending = None;
        assert_eq!(
            driver
                .attempt(
                    &mut cleanup,
                    &mut lease,
                    &mut pending,
                    Instant::now() + Duration::from_secs(2)
                )
                .await,
            Err(GatewayServiceCleanupDriverError::Stale)
        );
        assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn expired_cleaned_row_with_newer_fence_is_stale() {
        let (owner, lease, policy) = fixture();
        let provider = Arc::new(TestProvider {
            gate: None,
            started: None,
            calls: AtomicUsize::new(0),
        });
        let ownership = Arc::new(TestOwnership {
            renewals: AtomicUsize::new(0),
            mark_cleaned: AtomicUsize::new(0),
            renewal_lease: Mutex::new(lease.clone()),
            mark_result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let failures = Arc::new(TestFailures {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(())),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let mut newer = lease.clone();
        newer.fencing_token += 1;
        newer.state = GatewayServiceInstanceState::Cleaned;
        let targets = Arc::new(TestTargets {
            instance: Mutex::new(Some(newer)),
            responses: Mutex::new(Vec::new()),
        });
        let driver = driver_parts(
            &owner,
            policy,
            provider,
            ownership.clone(),
            failures,
            targets,
        );
        let mut cleanup =
            GatewayServiceCleanup::from_confirmed_physical(lease.identity, Duration::from_secs(1))
                .expect("cleanup");
        let mut lease = lease;
        let mut pending = None;
        assert_eq!(
            driver
                .attempt(
                    &mut cleanup,
                    &mut lease,
                    &mut pending,
                    Instant::now() - Duration::from_millis(1)
                )
                .await,
            Err(GatewayServiceCleanupDriverError::Stale)
        );
        assert_eq!(ownership.mark_cleaned.load(Ordering::Relaxed), 0);
    }
}

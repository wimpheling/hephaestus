//! Parent-owned boot recovery for abandoned persistent service instances.

use std::{future::Future, pin::Pin, sync::Arc, task::Poll, time::Duration};

use tokio::time::{self, Instant};
use vm_trait::VmProvider;

use crate::{
    GatewayServiceCleanup, GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError,
    GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverPolicy,
    GatewayServiceExpiredClaimRecovery, GatewayServiceFailure, GatewayServiceFailureStore,
    GatewayServiceInstanceLease, GatewayServiceInstancePage, GatewayServiceInstanceState,
    GatewayServiceLaunchResolver, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError, GatewayServiceTargetStore, MAX_SERVICE_INSTANCE_PAGE_SIZE,
};

/// Maximum number of cleanup attempts owned concurrently during boot recovery.
pub const MAX_SERVICE_BOOT_RECOVERY_CLEANUPS: usize = 2;
const BOOT_RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Immutable dependencies for the boot recovery gate.
pub struct GatewayServiceBootRecoveryContext {
    /// Stable host and daemon incarnation performing recovery.
    pub owner: GatewayServiceOwner,
    /// Lease and durable-operation bounds used by cleanup.
    pub cleanup_policy: GatewayServiceCleanupDriverPolicy,
    /// Physical/materializer cleanup timeout.
    pub shutdown_timeout: Duration,
    /// Durable ownership adapter for bounded batch discovery.
    pub ownership: Arc<dyn GatewayServiceOwnership>,
    /// Exact-instance takeover adapter for retained retries.
    pub exact_recovery: Arc<dyn GatewayServiceExpiredClaimRecovery>,
    /// Host inventory and exact-instance lookup adapter.
    pub targets: Arc<dyn GatewayServiceTargetStore>,
    /// Redacted durable failure store.
    pub failure_store: Arc<dyn GatewayServiceFailureStore>,
    /// Exact launch/materializer resolver.
    pub resolver: Arc<dyn GatewayServiceLaunchResolver>,
    /// VM provider used for orphan cleanup.
    pub provider: Arc<dyn VmProvider>,
}

/// Redacted boot-gate errors. Any error leaves the gate closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceBootRecoveryError {
    /// The immutable context or a durable lease was malformed.
    #[error("invalid gateway service boot recovery input")]
    InvalidInput,
    /// Durable recovery was unavailable; the caller may poll again.
    #[error("gateway service boot recovery is unavailable")]
    Unavailable,
}

/// One parent-visible boot recovery step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceBootRecoveryEvent {
    /// Recovery is waiting for an expired lease or a bounded retry delay.
    Waiting,
    /// One cleanup job finished but retained state for a later retry.
    Pending,
    /// No non-cleaned host rows remain after a fresh first-page proof.
    Complete,
}

/// Exact unresolved ownership returned when boot recovery is consumed.
pub struct GatewayServiceBootRecoveryUnresolved {
    /// Durable instance lease retained for later exact takeover.
    pub lease: GatewayServiceInstanceLease,
    /// Physical/materializer progress retained across attempts.
    pub cleanup: GatewayServiceCleanup,
    /// Failure report still awaiting durable recording.
    pub pending_failure: Option<GatewayServiceFailure>,
}

/// Result of consuming the boot gate after all owned futures settle.
pub struct GatewayServiceBootRecoveryShutdown {
    /// Claims and physical cleanup still requiring a later recovery pass.
    pub unresolved: Vec<GatewayServiceBootRecoveryUnresolved>,
    /// Raw claims retained when an adapter returned malformed ownership data.
    ///
    /// These rows are intentionally not converted into cleanup state: doing so
    /// would either invent a VM identity or discard durable ownership.
    pub unresolved_claims: Vec<GatewayServiceInstanceLease>,
}

struct BootCleanupState {
    lease: GatewayServiceInstanceLease,
    cleanup: GatewayServiceCleanup,
    pending_failure: Option<GatewayServiceFailure>,
}

struct ClaimCompletion {
    result: Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>,
    deadline: Instant,
    known_leases: Vec<GatewayServiceInstanceLease>,
}

struct CleanupCompletion {
    state: BootCleanupState,
    result: Result<GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverError>,
}

type ClaimFuture = Pin<Box<dyn Future<Output = ClaimCompletion> + Send>>;
type CleanupFuture = Pin<Box<dyn Future<Output = CleanupCompletion> + Send>>;

/// Parent-owned boot recovery gate.
pub struct GatewayServiceBootRecovery {
    context: Arc<GatewayServiceBootRecoveryContext>,
    claim_future: Option<ClaimFuture>,
    cleanup_jobs: Vec<CleanupFuture>,
    retained: Vec<BootCleanupState>,
    unresolved_claims: Vec<GatewayServiceInstanceLease>,
    blocked_on_malformed_claim: bool,
    next_retry_at: Option<Instant>,
    complete: bool,
}

impl GatewayServiceBootRecovery {
    /// Creates a boot gate with validated immutable dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceBootRecoveryError::InvalidInput`] for an
    /// invalid owner, cleanup policy, or timeout.
    pub fn new(
        context: GatewayServiceBootRecoveryContext,
    ) -> Result<Self, GatewayServiceBootRecoveryError> {
        context
            .owner
            .validate()
            .map_err(|_| GatewayServiceBootRecoveryError::InvalidInput)?;
        context
            .cleanup_policy
            .validate()
            .map_err(|_| GatewayServiceBootRecoveryError::InvalidInput)?;
        if context.shutdown_timeout.is_zero()
            || context.shutdown_timeout > crate::service_instance::MAX_SHUTDOWN_TIMEOUT
        {
            return Err(GatewayServiceBootRecoveryError::InvalidInput);
        }
        Ok(Self {
            context: Arc::new(context),
            claim_future: None,
            cleanup_jobs: Vec::new(),
            retained: Vec::new(),
            unresolved_claims: Vec::new(),
            blocked_on_malformed_claim: false,
            next_retry_at: None,
            complete: false,
        })
    }

    /// Returns whether the gate has proven an empty fresh host inventory.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// Returns whether parent-owned claim or cleanup futures remain.
    #[must_use]
    pub fn has_pending_work(&self) -> bool {
        self.claim_future.is_some() || !self.cleanup_jobs.is_empty()
    }

    /// Waits for one bounded recovery step.
    ///
    /// Dropping this wait never drops a claim or cleanup future. The caller
    /// must continue polling or consume the gate through [`Self::shutdown`].
    ///
    /// # Errors
    ///
    /// Returns a redacted unavailable or invalid-input result while retaining
    /// all unresolved state and keeping the gate closed.
    #[allow(clippy::too_many_lines)] // One poll loop owns every raw claim and cleanup future.
    pub async fn poll(
        &mut self,
    ) -> Result<GatewayServiceBootRecoveryEvent, GatewayServiceBootRecoveryError> {
        loop {
            if let Some(completion) = poll_cleanup_jobs(&mut self.cleanup_jobs).await {
                self.finish_cleanup(completion);
                if self.cleanup_jobs.is_empty() {
                    self.next_retry_at = Some(Instant::now() + BOOT_RECOVERY_RETRY_DELAY);
                }
                return Ok(GatewayServiceBootRecoveryEvent::Pending);
            }
            if let Some(next) = self.next_retry_at {
                if next > Instant::now() {
                    time::sleep_until(next).await;
                }
                self.next_retry_at = None;
            }
            if self.schedule_retained_retries() {
                continue;
            }
            if let Some(claim_future) = self.claim_future.as_mut() {
                let completion = claim_future.await;
                self.claim_future = None;
                let has_leases = matches!(&completion.result, Ok(leases) if !leases.is_empty());
                self.finish_claim(completion)?;
                if !has_leases {
                    self.next_retry_at = Some(Instant::now() + BOOT_RECOVERY_RETRY_DELAY);
                    return Ok(GatewayServiceBootRecoveryEvent::Waiting);
                }
                return Ok(GatewayServiceBootRecoveryEvent::Pending);
            }
            if self.complete {
                return Ok(GatewayServiceBootRecoveryEvent::Complete);
            }
            if self.blocked_on_malformed_claim {
                self.next_retry_at = Some(Instant::now() + BOOT_RECOVERY_RETRY_DELAY);
                return Ok(GatewayServiceBootRecoveryEvent::Waiting);
            }
            if !self.cleanup_jobs.is_empty() || !self.retained.is_empty() {
                continue;
            }
            let page = GatewayServiceInstancePage::new(
                self.context.owner.host_id.clone(),
                None,
                MAX_SERVICE_INSTANCE_PAGE_SIZE,
            )
            .map_err(|_| GatewayServiceBootRecoveryError::InvalidInput)?;
            let inventory = self
                .context
                .targets
                .list_service_instances(page)
                .await
                .map_err(|_| {
                    self.next_retry_at = Some(Instant::now() + BOOT_RECOVERY_RETRY_DELAY);
                    GatewayServiceBootRecoveryError::Unavailable
                })?;
            if inventory.instances.is_empty() {
                self.complete = true;
                return Ok(GatewayServiceBootRecoveryEvent::Complete);
            }
            let available = MAX_SERVICE_BOOT_RECOVERY_CLEANUPS
                .saturating_sub(self.cleanup_jobs.len())
                .saturating_sub(self.retained.len());
            if available == 0 {
                continue;
            }
            let started = Instant::now();
            let deadline = started
                .checked_add(self.context.cleanup_policy.lease.lease_duration)
                .ok_or(GatewayServiceBootRecoveryError::InvalidInput)?;
            let ownership = Arc::clone(&self.context.ownership);
            let owner = self.context.owner.clone();
            let lease_duration = self.context.cleanup_policy.lease.lease_duration;
            let limit = available.min(MAX_SERVICE_BOOT_RECOVERY_CLEANUPS);
            let own_stopping = inventory
                .instances
                .iter()
                .filter(|lease| {
                    lease.owner_host_id == self.context.owner.host_id
                        && lease.owner_uuid == self.context.owner.owner_uuid
                        && lease.state == GatewayServiceInstanceState::Stopping
                })
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            self.claim_future = Some(Box::pin(async move {
                if own_stopping.is_empty() {
                    let result = ownership.claim_expired(&owner, lease_duration, limit).await;
                    ClaimCompletion {
                        result,
                        deadline,
                        known_leases: Vec::new(),
                    }
                } else {
                    let mut leases = Vec::with_capacity(own_stopping.len());
                    for candidate in own_stopping {
                        match ownership.renew(&candidate, &owner, lease_duration).await {
                            Ok(lease) if valid_owned_recovery_lease(&lease, &owner, &candidate) => {
                                leases.push(lease);
                            }
                            Ok(_) | Err(GatewayServiceOwnershipError::StaleLease) => {}
                            Err(error) => {
                                return ClaimCompletion {
                                    result: Err(error),
                                    deadline,
                                    known_leases: leases,
                                };
                            }
                        }
                    }
                    ClaimCompletion {
                        result: Ok(leases),
                        deadline,
                        known_leases: Vec::new(),
                    }
                }
            }));
        }
    }

    /// Consumes the gate after settling all currently owned futures.
    pub async fn shutdown(mut self) -> GatewayServiceBootRecoveryShutdown {
        while self.claim_future.is_some() || !self.cleanup_jobs.is_empty() {
            if let Some(claim_future) = self.claim_future.as_mut() {
                let completion = claim_future.await;
                self.claim_future = None;
                let _ = self.finish_claim(completion);
                continue;
            }
            if let Some(completion) = poll_cleanup_jobs(&mut self.cleanup_jobs).await {
                self.finish_cleanup(completion);
            }
        }
        GatewayServiceBootRecoveryShutdown {
            unresolved: self
                .retained
                .into_iter()
                .map(|state| GatewayServiceBootRecoveryUnresolved {
                    lease: state.lease,
                    cleanup: state.cleanup,
                    pending_failure: state.pending_failure,
                })
                .collect(),
            unresolved_claims: self.unresolved_claims,
        }
    }

    fn schedule_retained_retries(&mut self) -> bool {
        let available = MAX_SERVICE_BOOT_RECOVERY_CLEANUPS.saturating_sub(self.cleanup_jobs.len());
        if available == 0 || self.retained.is_empty() {
            return false;
        }
        let count = available.min(self.retained.len());
        let states = self.retained.drain(..count).collect::<Vec<_>>();
        for state in states {
            let context = Arc::clone(&self.context);
            self.cleanup_jobs.push(Box::pin(run_retry(context, state)));
        }
        true
    }

    fn finish_claim(
        &mut self,
        completion: ClaimCompletion,
    ) -> Result<(), GatewayServiceBootRecoveryError> {
        let Ok(leases) = completion.result else {
            let known = completion.known_leases;
            if !known.is_empty() {
                self.enqueue_claim_leases(known, completion.deadline)?;
            }
            self.next_retry_at = Some(Instant::now() + BOOT_RECOVERY_RETRY_DELAY);
            return Err(GatewayServiceBootRecoveryError::Unavailable);
        };
        self.enqueue_claim_leases(leases, completion.deadline)
    }

    fn enqueue_claim_leases(
        &mut self,
        leases: Vec<GatewayServiceInstanceLease>,
        deadline: Instant,
    ) -> Result<(), GatewayServiceBootRecoveryError> {
        if leases.len() > MAX_SERVICE_BOOT_RECOVERY_CLEANUPS {
            self.unresolved_claims.extend(leases);
            self.blocked_on_malformed_claim = true;
            return Err(GatewayServiceBootRecoveryError::InvalidInput);
        }
        if leases
            .iter()
            .any(|lease| !valid_recovery_lease(lease, &self.context.owner))
        {
            self.unresolved_claims.extend(leases);
            self.blocked_on_malformed_claim = true;
            return Err(GatewayServiceBootRecoveryError::InvalidInput);
        }
        for lease in leases {
            let Ok(cleanup) =
                GatewayServiceCleanup::new(lease.identity, None, self.context.shutdown_timeout)
            else {
                self.unresolved_claims.push(lease);
                return Err(GatewayServiceBootRecoveryError::InvalidInput);
            };
            let state = BootCleanupState {
                lease,
                cleanup,
                pending_failure: None,
            };
            self.cleanup_jobs.push(Box::pin(run_cleanup(
                Arc::clone(&self.context),
                state,
                deadline,
            )));
        }
        Ok(())
    }

    fn finish_cleanup(&mut self, completion: CleanupCompletion) {
        if !matches!(
            completion.result,
            Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
        ) {
            self.retained.push(completion.state);
        }
    }
}

async fn run_cleanup(
    context: Arc<GatewayServiceBootRecoveryContext>,
    mut state: BootCleanupState,
    deadline: Instant,
) -> CleanupCompletion {
    let result = match GatewayServiceCleanupDriver::new(
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.targets),
        Arc::clone(&context.provider),
        Arc::clone(&context.resolver),
        context.owner.clone(),
        context.cleanup_policy,
    ) {
        Ok(driver) => {
            driver
                .attempt(
                    &mut state.cleanup,
                    &mut state.lease,
                    &mut state.pending_failure,
                    deadline,
                )
                .await
        }
        Err(error) => Err(error),
    };
    CleanupCompletion { state, result }
}

async fn run_retry(
    context: Arc<GatewayServiceBootRecoveryContext>,
    mut state: BootCleanupState,
) -> CleanupCompletion {
    let call_started = Instant::now();
    let deadline = call_started
        .checked_add(context.cleanup_policy.lease.lease_duration)
        .unwrap_or(call_started);
    let result = match cleanup_driver(&context) {
        Ok(driver)
            if state.cleanup.vm_teardown_confirmed()
                && state.cleanup.materializer_cleanup_confirmed() =>
        {
            match driver
                .confirm_cleaned_state(&state.cleanup, &state.lease)
                .await
            {
                Ok(true) => Ok(GatewayServiceCleanupDriverOutcome::Cleaned),
                Ok(false) => retry_takeover(&context, &mut state, deadline, driver).await,
                Err(error) => Err(error),
            }
        }
        Ok(driver) => retry_takeover(&context, &mut state, deadline, driver).await,
        Err(error) => Err(error),
    };
    CleanupCompletion { state, result }
}

async fn retry_takeover(
    context: &GatewayServiceBootRecoveryContext,
    state: &mut BootCleanupState,
    deadline: Instant,
    driver: GatewayServiceCleanupDriver,
) -> Result<GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverError> {
    let old_lease = state.lease.clone();
    let takeover = context.exact_recovery.claim_expired_instance(
        &old_lease,
        &context.owner,
        context.cleanup_policy.lease.lease_duration,
    );
    match takeover.await {
        Ok(lease) if valid_exact_takeover_lease(&lease, &context.owner, &old_lease) => {
            state.lease = lease;
            driver
                .attempt(
                    &mut state.cleanup,
                    &mut state.lease,
                    &mut state.pending_failure,
                    deadline,
                )
                .await
        }
        Ok(_) => Err(GatewayServiceCleanupDriverError::Stale),
        Err(GatewayServiceOwnershipError::StaleLease) => {
            Err(GatewayServiceCleanupDriverError::Stale)
        }
        Err(_) => Err(GatewayServiceCleanupDriverError::Unavailable),
    }
}

fn cleanup_driver(
    context: &GatewayServiceBootRecoveryContext,
) -> Result<GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError> {
    GatewayServiceCleanupDriver::new(
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.targets),
        Arc::clone(&context.provider),
        Arc::clone(&context.resolver),
        context.owner.clone(),
        context.cleanup_policy,
    )
}

async fn poll_cleanup_jobs(jobs: &mut Vec<CleanupFuture>) -> Option<CleanupCompletion> {
    std::future::poll_fn(|context| {
        for index in (0..jobs.len()).rev() {
            if let Poll::Ready(completion) = jobs[index].as_mut().poll(context) {
                drop(jobs.swap_remove(index));
                return Poll::Ready(Some(completion));
            }
        }
        if jobs.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    })
    .await
}

fn valid_recovery_lease(lease: &GatewayServiceInstanceLease, owner: &GatewayServiceOwner) -> bool {
    let same_host = lease.owner_host_id == owner.host_id;
    !lease.identity.instance_id.is_nil()
        && !lease.identity.gateway_id.is_nil()
        && !lease.identity.revision_id.is_nil()
        && same_host
        && lease.owner_uuid == owner.owner_uuid
        && lease.fencing_token > 0
        && lease.vm_id == format!("gateway-service-{}", lease.identity.instance_id)
        && lease.state == GatewayServiceInstanceState::Stopping
        && lease.lease_expires_at > lease.heartbeat_at
}

fn valid_owned_recovery_lease(
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    expected: &GatewayServiceInstanceLease,
) -> bool {
    valid_recovery_lease(lease, owner)
        && lease.identity == expected.identity
        && lease.vm_id == expected.vm_id
        && lease.fencing_token == expected.fencing_token
}

fn valid_exact_takeover_lease(
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    previous: &GatewayServiceInstanceLease,
) -> bool {
    valid_recovery_lease(lease, owner)
        && lease.identity == previous.identity
        && lease.vm_id == previous.vm_id
        && lease.fencing_token > previous.fencing_token
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayServiceIdentity;
    use ::time::{Duration as TimeDuration, OffsetDateTime};
    use async_trait::async_trait;
    use std::collections::{HashSet, VecDeque};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;
    use uuid::Uuid;
    use vm_trait::{VmError, VmId, VmInstance, VmSpec};

    struct TestOwnership {
        claim_started: Option<Arc<Notify>>,
        claim_release: Option<Arc<Notify>>,
        renew_own: bool,
        renew_calls: AtomicUsize,
        claim_batches: Mutex<VecDeque<Vec<GatewayServiceInstanceLease>>>,
        claim_limits: Mutex<Vec<usize>>,
        accept_cleaned: bool,
        cleaned: Option<Arc<Mutex<HashSet<Uuid>>>>,
        renewed_instances: Arc<Mutex<HashSet<Uuid>>>,
        renew_fail_after: Option<usize>,
    }

    impl TestOwnership {
        fn standard() -> Self {
            Self {
                claim_started: None,
                claim_release: None,
                renew_own: false,
                renew_calls: AtomicUsize::new(0),
                claim_batches: Mutex::new(VecDeque::new()),
                claim_limits: Mutex::new(Vec::new()),
                accept_cleaned: false,
                cleaned: None,
                renewed_instances: Arc::new(Mutex::new(HashSet::new())),
                renew_fail_after: None,
            }
        }

        fn blocking(started: Arc<Notify>, release: Arc<Notify>) -> Self {
            Self {
                claim_started: Some(started),
                claim_release: Some(release),
                ..Self::standard()
            }
        }

        fn renew_own() -> Self {
            Self {
                renew_own: true,
                ..Self::standard()
            }
        }

        fn batched_with_cleanup_state(
            batches: Vec<Vec<GatewayServiceInstanceLease>>,
            cleaned: Arc<Mutex<HashSet<Uuid>>>,
        ) -> Self {
            Self {
                claim_batches: Mutex::new(batches.into()),
                accept_cleaned: true,
                cleaned: Some(cleaned),
                ..Self::standard()
            }
        }
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
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.renew_calls.fetch_add(1, Ordering::Relaxed);
            let count = self.renew_calls.load(Ordering::Relaxed);
            if self.renew_fail_after.is_some_and(|limit| count > limit) {
                return Err(GatewayServiceOwnershipError::Unavailable);
            }
            if self.renew_own {
                self.renewed_instances
                    .lock()
                    .expect("renewed instances")
                    .insert(lease.identity.instance_id);
            }
            self.renew_own
                .then(|| lease.clone())
                .ok_or(GatewayServiceOwnershipError::Unavailable)
        }

        async fn claim_expired(
            &self,
            _: &GatewayServiceOwner,
            _: Duration,
            limit: usize,
        ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
            self.claim_limits.lock().expect("claim limits").push(limit);
            if let Some(started) = &self.claim_started {
                started.notify_waiters();
            }
            if let Some(release) = &self.claim_release {
                release.notified().await;
            }
            Ok(self
                .claim_batches
                .lock()
                .expect("claim batches")
                .pop_front()
                .unwrap_or_default())
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
            lease: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<(), GatewayServiceOwnershipError> {
            if let Some(cleaned) = &self.cleaned {
                cleaned
                    .lock()
                    .expect("cleaned")
                    .insert(lease.identity.instance_id);
            }
            self.accept_cleaned
                .then_some(())
                .ok_or(GatewayServiceOwnershipError::Unavailable)
        }
    }

    struct TestExactRecovery;

    #[async_trait]
    impl GatewayServiceExpiredClaimRecovery for TestExactRecovery {
        async fn claim_expired_instance(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Err(GatewayServiceOwnershipError::Unavailable)
        }
    }

    struct TestTargets {
        instances: Mutex<Vec<GatewayServiceInstanceLease>>,
        inventory: Mutex<Vec<GatewayServiceInstanceLease>>,
        list_calls: AtomicUsize,
        hide_after: Option<usize>,
        cleaned: Option<Arc<Mutex<HashSet<Uuid>>>>,
    }

    #[async_trait]
    impl GatewayServiceTargetStore for TestTargets {
        async fn list_service_targets(
            &self,
            _: crate::GatewayServiceTargetPage,
        ) -> Result<crate::GatewayServiceTargetPageResult, crate::GatewayEdgeError> {
            Ok(crate::GatewayServiceTargetPageResult {
                targets: Vec::new(),
                next_after: None,
            })
        }

        async fn get_service_target(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<Option<crate::GatewayServiceOwnedTarget>, crate::GatewayEdgeError> {
            Ok(None)
        }

        async fn count_accepted_service_invocations(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<u64, crate::GatewayEdgeError> {
            Ok(0)
        }

        async fn count_accepted_service_invocations_for_instance(
            &self,
            _: crate::GatewayServiceInstanceKey,
        ) -> Result<u64, crate::GatewayEdgeError> {
            Ok(0)
        }

        async fn get_service_instance(
            &self,
            identity: GatewayServiceIdentity,
        ) -> Result<Option<GatewayServiceInstanceLease>, crate::GatewayEdgeError> {
            Ok(self
                .instances
                .lock()
                .expect("instances")
                .iter()
                .find(|lease| lease.identity == identity)
                .cloned())
        }

        async fn list_service_instances(
            &self,
            _: crate::GatewayServiceInstancePage,
        ) -> Result<crate::GatewayServiceInstancePageResult, crate::GatewayEdgeError> {
            let call = self.list_calls.fetch_add(1, Ordering::Relaxed) + 1;
            let instances = if self.hide_after.is_some_and(|limit| call > limit) {
                Vec::new()
            } else {
                self.inventory
                    .lock()
                    .expect("inventory")
                    .iter()
                    .filter(|lease| {
                        self.cleaned.as_ref().is_none_or(|cleaned| {
                            !cleaned
                                .lock()
                                .expect("cleaned")
                                .contains(&lease.identity.instance_id)
                        })
                    })
                    .cloned()
                    .collect()
            };
            Ok(crate::GatewayServiceInstancePageResult {
                instances,
                next_after: None,
            })
        }
    }

    struct TestFailureStore;

    #[async_trait]
    impl GatewayServiceFailureStore for TestFailureStore {
        async fn record_failure(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: GatewayServiceFailure,
        ) -> Result<(), crate::GatewayServiceFailureStoreError> {
            Ok(())
        }
    }

    struct TestResolver;

    #[async_trait]
    impl GatewayServiceLaunchResolver for TestResolver {
        async fn resolve_service_launch(
            &self,
            _: crate::GatewayServiceLaunchRequest,
        ) -> Result<crate::GatewayServiceLaunch, crate::GatewayEdgeError> {
            Err(crate::GatewayEdgeError::Unavailable)
        }

        async fn cleanup_service_launch(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<(), crate::GatewayEdgeError> {
            Ok(())
        }
    }

    struct TestProvider {
        physical: Option<Arc<Mutex<HashSet<String>>>>,
        block_started: Option<Arc<Notify>>,
        block_release: Option<Arc<Notify>>,
        block_count: Option<Arc<AtomicUsize>>,
    }

    impl TestProvider {
        fn standard() -> Self {
            Self {
                physical: None,
                block_started: None,
                block_release: None,
                block_count: None,
            }
        }
    }

    #[async_trait]
    impl VmProvider for TestProvider {
        fn name(&self) -> &'static str {
            "boot-recovery-test"
        }

        async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
            Err(VmError::Unavailable {
                resource: String::from("test provider"),
                reason: String::from("not used"),
            })
        }

        async fn cleanup_orphan(&self, vm_id: &VmId) -> Result<(), VmError> {
            if let Some(physical) = &self.physical {
                physical.lock().expect("physical").insert(vm_id.0.clone());
            }
            if let (Some(started), Some(release), Some(count)) =
                (&self.block_started, &self.block_release, &self.block_count)
            {
                if count.fetch_add(1, Ordering::Relaxed) + 1 == 2 {
                    started.notify_waiters();
                }
                release.notified().await;
            }
            Ok(())
        }
    }

    fn context_with_owner(
        instances: Vec<GatewayServiceInstanceLease>,
        owner: GatewayServiceOwner,
        ownership: Arc<TestOwnership>,
    ) -> GatewayServiceBootRecoveryContext {
        let targets = Arc::new(TestTargets {
            inventory: Mutex::new(instances.clone()),
            instances: Mutex::new(instances),
            list_calls: AtomicUsize::new(0),
            hide_after: None,
            cleaned: None,
        });
        context_with_targets_and_provider(
            owner,
            ownership,
            targets,
            Arc::new(TestProvider::standard()),
        )
    }

    fn context_with_targets_and_provider(
        owner: GatewayServiceOwner,
        ownership: Arc<TestOwnership>,
        targets: Arc<TestTargets>,
        provider: Arc<TestProvider>,
    ) -> GatewayServiceBootRecoveryContext {
        let policy = crate::GatewayServiceSupervisorPolicy::default();
        GatewayServiceBootRecoveryContext {
            owner,
            cleanup_policy: GatewayServiceCleanupDriverPolicy {
                lease: policy.lease,
                database_timeout: policy.instance.probe_timeout,
            },
            shutdown_timeout: policy.instance.shutdown_timeout,
            ownership,
            exact_recovery: Arc::new(TestExactRecovery),
            targets,
            failure_store: Arc::new(TestFailureStore),
            resolver: Arc::new(TestResolver),
            provider,
        }
    }

    fn context(instances: Vec<GatewayServiceInstanceLease>) -> GatewayServiceBootRecoveryContext {
        context_with_owner(
            instances,
            GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner"),
            Arc::new(TestOwnership::standard()),
        )
    }

    fn inventory_lease() -> GatewayServiceInstanceLease {
        let identity = GatewayServiceIdentity {
            instance_id: Uuid::new_v4(),
            gateway_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        };
        let now = OffsetDateTime::now_utc();
        GatewayServiceInstanceLease {
            identity,
            owner_host_id: String::from("boot-test-host"),
            owner_uuid: Uuid::new_v4(),
            fencing_token: 1,
            state: GatewayServiceInstanceState::Provisioning,
            vm_id: format!("gateway-service-{}", identity.instance_id),
            lease_expires_at: now + TimeDuration::minutes(1),
            heartbeat_at: now,
        }
    }

    #[tokio::test]
    async fn empty_first_inventory_page_proves_complete() {
        let mut recovery = GatewayServiceBootRecovery::new(context(Vec::new())).expect("gate");
        assert_eq!(
            recovery.poll().await.expect("empty proof"),
            GatewayServiceBootRecoveryEvent::Complete
        );
        assert!(recovery.is_complete());
    }

    #[tokio::test]
    async fn unexpired_inventory_keeps_gate_closed() {
        let mut recovery =
            GatewayServiceBootRecovery::new(context(vec![inventory_lease()])).expect("gate");
        assert_eq!(
            recovery.poll().await.expect("bounded wait"),
            GatewayServiceBootRecoveryEvent::Waiting
        );
        assert!(!recovery.is_complete());
        assert!(!recovery.has_pending_work());
    }

    #[tokio::test]
    async fn own_stopping_inventory_is_renewed_before_cleanup() {
        let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
        let ownership = Arc::new(TestOwnership::renew_own());
        let mut lease = inventory_lease();
        lease.owner_host_id = owner.host_id.clone();
        lease.owner_uuid = owner.owner_uuid;
        lease.state = GatewayServiceInstanceState::Stopping;
        let identity = lease.identity;
        let mut recovery = GatewayServiceBootRecovery::new(context_with_owner(
            vec![lease],
            owner,
            Arc::clone(&ownership),
        ))
        .expect("gate");

        assert_eq!(
            recovery.poll().await.expect("renew own row"),
            GatewayServiceBootRecoveryEvent::Pending
        );
        assert_eq!(ownership.renew_calls.load(Ordering::Relaxed), 1);
        let shutdown = recovery.shutdown().await;
        assert_eq!(shutdown.unresolved.len(), 1);
        assert_eq!(shutdown.unresolved[0].lease.identity, identity);
    }

    #[tokio::test]
    async fn partial_renewal_failure_keeps_successful_claim_bounded_and_owned() {
        let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
        let mut leases = Vec::new();
        for _ in 0..2 {
            let mut lease = inventory_lease();
            lease.owner_host_id = owner.host_id.clone();
            lease.owner_uuid = owner.owner_uuid;
            lease.state = GatewayServiceInstanceState::Stopping;
            leases.push(lease);
        }
        let mut ownership = TestOwnership::renew_own();
        ownership.renew_fail_after = Some(1);
        let ownership = Arc::new(ownership);
        let mut recovery = GatewayServiceBootRecovery::new(context_with_owner(
            leases,
            owner,
            Arc::clone(&ownership),
        ))
        .expect("gate");

        assert_eq!(
            recovery.poll().await,
            Err(GatewayServiceBootRecoveryError::Unavailable)
        );
        assert!(recovery.unresolved_claims.is_empty());
        assert_eq!(recovery.cleanup_jobs.len(), 1);
        assert!(!recovery.is_complete());
        let shutdown = recovery.shutdown().await;
        assert!(shutdown.unresolved_claims.is_empty());
        assert_eq!(shutdown.unresolved.len(), 1);
    }

    #[tokio::test]
    async fn backlog_is_drained_in_two_claim_slots_before_fresh_empty_proof() {
        let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
        let mut leases = Vec::new();
        for _ in 0..12 {
            let mut lease = inventory_lease();
            lease.owner_host_id = owner.host_id.clone();
            lease.owner_uuid = owner.owner_uuid;
            lease.state = GatewayServiceInstanceState::Stopping;
            leases.push(lease);
        }
        let batches = leases.chunks(2).map(<[_]>::to_vec).collect::<Vec<_>>();
        let cleaned = Arc::new(Mutex::new(HashSet::new()));
        let inventory = leases
            .iter()
            .cloned()
            .map(|mut lease| {
                lease.owner_uuid = Uuid::new_v4();
                lease.state = GatewayServiceInstanceState::Provisioning;
                lease
            })
            .collect();
        let ownership = Arc::new(TestOwnership::batched_with_cleanup_state(
            batches,
            Arc::clone(&cleaned),
        ));
        let physical = Arc::new(Mutex::new(HashSet::new()));
        let targets = Arc::new(TestTargets {
            instances: Mutex::new(leases.clone()),
            inventory: Mutex::new(inventory),
            list_calls: AtomicUsize::new(0),
            hide_after: None,
            cleaned: Some(Arc::clone(&cleaned)),
        });
        let mut recovery = GatewayServiceBootRecovery::new(context_with_targets_and_provider(
            owner,
            Arc::clone(&ownership),
            targets,
            Arc::new(TestProvider {
                physical: Some(Arc::clone(&physical)),
                ..TestProvider::standard()
            }),
        ))
        .expect("gate");

        let mut completed = false;
        for _ in 0..20 {
            if tokio::time::timeout(std::time::Duration::from_secs(3), recovery.poll())
                .await
                .expect("bounded boot step")
                .expect("recovery step")
                == GatewayServiceBootRecoveryEvent::Complete
            {
                completed = true;
                break;
            }
        }
        assert!(completed, "backlog must reach a fresh empty proof");
        assert_eq!(cleaned.lock().expect("cleaned").len(), 12);
        assert_eq!(physical.lock().expect("physical").len(), 12);
        assert!(
            ownership
                .claim_limits
                .lock()
                .expect("claim limits")
                .iter()
                .all(|limit| *limit <= MAX_SERVICE_BOOT_RECOVERY_CLEANUPS)
        );
        assert_eq!(
            ownership.claim_limits.lock().expect("claim limits").len(),
            6
        );
    }

    #[tokio::test]
    async fn blocked_cleanup_does_not_starve_another_owned_slot() {
        let first = inventory_lease();
        let second = inventory_lease();
        let first_state = BootCleanupState {
            cleanup: GatewayServiceCleanup::new(first.identity, None, Duration::from_secs(1))
                .expect("cleanup"),
            lease: first.clone(),
            pending_failure: None,
        };
        let second_state = BootCleanupState {
            cleanup: GatewayServiceCleanup::new(second.identity, None, Duration::from_secs(1))
                .expect("cleanup"),
            lease: second.clone(),
            pending_failure: None,
        };
        let blocker = Box::pin(async move {
            std::future::pending::<()>().await;
            CleanupCompletion {
                state: first_state,
                result: Ok(GatewayServiceCleanupDriverOutcome::Pending {
                    physical_complete: false,
                    failure_recorded: false,
                    lease_lost: false,
                }),
            }
        });
        let ready = Box::pin(async move {
            CleanupCompletion {
                state: second_state,
                result: Err(GatewayServiceCleanupDriverError::Unavailable),
            }
        });
        let mut jobs: Vec<CleanupFuture> = vec![blocker, ready];
        let completion = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            poll_cleanup_jobs(&mut jobs),
        )
        .await
        .expect("ready cleanup must win");
        assert!(completion.is_some());
        assert_eq!(jobs.len(), 1);
    }

    #[tokio::test]
    async fn both_cleanup_slots_renew_while_physical_teardown_is_blocked() {
        let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
        let mut leases = Vec::new();
        for _ in 0..2 {
            let mut lease = inventory_lease();
            lease.owner_host_id = owner.host_id.clone();
            lease.owner_uuid = owner.owner_uuid;
            lease.state = GatewayServiceInstanceState::Stopping;
            leases.push(lease);
        }
        let batches = vec![leases.clone()];
        let cleaned = Arc::new(Mutex::new(HashSet::new()));
        let mut ownership =
            TestOwnership::batched_with_cleanup_state(batches, Arc::clone(&cleaned));
        ownership.renew_own = true;
        let ownership = Arc::new(ownership);
        let inventory = leases
            .iter()
            .cloned()
            .map(|mut lease| {
                lease.owner_uuid = Uuid::new_v4();
                lease.state = GatewayServiceInstanceState::Provisioning;
                lease
            })
            .collect();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let started_count = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(TestProvider {
            physical: None,
            block_started: Some(Arc::clone(&started)),
            block_release: Some(Arc::clone(&release)),
            block_count: Some(Arc::clone(&started_count)),
        });
        let targets = Arc::new(TestTargets {
            instances: Mutex::new(leases.clone()),
            inventory: Mutex::new(inventory),
            list_calls: AtomicUsize::new(0),
            hide_after: None,
            cleaned: Some(Arc::clone(&cleaned)),
        });
        let mut context =
            context_with_targets_and_provider(owner, Arc::clone(&ownership), targets, provider);
        context.cleanup_policy.lease.lease_duration = Duration::from_millis(300);
        context.cleanup_policy.lease.renewal_interval = Duration::from_millis(20);
        let mut recovery = GatewayServiceBootRecovery::new(context).expect("gate");
        assert_eq!(
            recovery.poll().await.expect("claim"),
            GatewayServiceBootRecoveryEvent::Pending
        );

        {
            let step = recovery.poll();
            tokio::pin!(step);
            tokio::select! {
                () = started.notified() => {}
                result = &mut step => panic!("cleanup should remain blocked: {result:?}"),
            }
            let mut renewed = false;
            for _ in 0..20 {
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(20)) => {
                        if ownership.renew_calls.load(Ordering::Relaxed) >= 2 {
                            renewed = true;
                            break;
                        }
                    }
                    result = &mut step => panic!("cleanup completed before release: {result:?}"),
                }
            }
            assert!(renewed, "both blocked cleanups must continue lease renewal");
            let renewed_instances = ownership
                .renewed_instances
                .lock()
                .expect("renewed instances")
                .clone();
            assert_eq!(renewed_instances.len(), 2);
            assert!(
                leases
                    .iter()
                    .all(|lease| { renewed_instances.contains(&lease.identity.instance_id) })
            );
            release.notify_waiters();
            let event = tokio::time::timeout(Duration::from_secs(1), &mut step)
                .await
                .expect("cleanup settles")
                .expect("cleanup event");
            assert_eq!(event, GatewayServiceBootRecoveryEvent::Pending);
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), recovery.poll())
            .await
            .expect("second cleanup settles")
            .expect("second cleanup event");
        assert_eq!(cleaned.lock().expect("cleaned").len(), 2);
    }

    #[tokio::test]
    async fn dropped_poll_keeps_claim_owned_until_shutdown() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let ownership = Arc::new(TestOwnership::blocking(
            Arc::clone(&started),
            Arc::clone(&release),
        ));
        let mut recovery = GatewayServiceBootRecovery::new(context_with_owner(
            vec![inventory_lease()],
            GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner"),
            ownership,
        ))
        .expect("gate");
        {
            let poll = recovery.poll();
            tokio::pin!(poll);
            tokio::select! {
                _ = &mut poll => panic!("claim should remain parent-owned"),
                () = started.notified() => {}
            }
        }
        assert!(recovery.has_pending_work());
        release.notify_waiters();
        let shutdown = tokio::time::timeout(std::time::Duration::from_secs(1), recovery.shutdown())
            .await
            .expect("shutdown settles claim");
        assert!(shutdown.unresolved.is_empty());
        assert!(shutdown.unresolved_claims.is_empty());
    }

    #[tokio::test]
    async fn malformed_batch_claim_is_retained_for_later_resolution() {
        let mut recovery = GatewayServiceBootRecovery::new(context(Vec::new())).expect("gate");
        let malformed = inventory_lease();
        let identity = malformed.identity;
        recovery
            .finish_claim(ClaimCompletion {
                result: Ok(vec![malformed]),
                deadline: Instant::now(),
                known_leases: Vec::new(),
            })
            .expect_err("malformed claim must close the gate");
        let shutdown = recovery.shutdown().await;
        assert_eq!(shutdown.unresolved_claims.len(), 1);
        assert_eq!(shutdown.unresolved_claims[0].identity, identity);
    }

    #[tokio::test]
    async fn malformed_batch_blocks_fresh_empty_completion_and_keeps_all_claims() {
        let context = context(Vec::new());
        let mut valid = inventory_lease();
        valid.owner_host_id = context.owner.host_id.clone();
        valid.owner_uuid = context.owner.owner_uuid;
        valid.state = GatewayServiceInstanceState::Stopping;
        let malformed = inventory_lease();
        let mut recovery = GatewayServiceBootRecovery::new(context).expect("gate");
        recovery
            .finish_claim(ClaimCompletion {
                result: Ok(vec![malformed, valid]),
                deadline: Instant::now(),
                known_leases: Vec::new(),
            })
            .expect_err("mixed malformed batch must close the gate");
        assert_eq!(
            recovery.poll().await.expect("closed gate"),
            GatewayServiceBootRecoveryEvent::Waiting
        );
        assert!(!recovery.is_complete());
        let shutdown = recovery.shutdown().await;
        assert_eq!(shutdown.unresolved_claims.len(), 2);
    }

    #[test]
    fn exact_retry_rejects_a_different_identity_or_fence() {
        let owner = GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner");
        let mut expected = inventory_lease();
        expected.owner_host_id = owner.host_id.clone();
        expected.owner_uuid = owner.owner_uuid;
        expected.state = GatewayServiceInstanceState::Stopping;
        let mut returned = expected.clone();
        assert!(valid_owned_recovery_lease(&returned, &owner, &expected));
        assert!(!valid_exact_takeover_lease(&returned, &owner, &expected));
        returned.fencing_token = expected.fencing_token + 1;
        assert!(valid_exact_takeover_lease(&returned, &owner, &expected));
        returned.identity.instance_id = Uuid::new_v4();
        returned.vm_id = format!("gateway-service-{}", returned.identity.instance_id);
        assert!(!valid_owned_recovery_lease(&returned, &owner, &expected));
    }
}

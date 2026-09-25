use super::helpers::{
    poll_cleanup_jobs, run_cleanup, run_retry, valid_owned_recovery_lease, valid_recovery_lease,
};
use super::{
    BOOT_RECOVERY_RETRY_DELAY, BootCleanupState, ClaimCompletion, CleanupCompletion,
    GatewayServiceBootRecovery, GatewayServiceBootRecoveryContext, GatewayServiceBootRecoveryError,
    GatewayServiceBootRecoveryEvent, GatewayServiceBootRecoveryShutdown,
    GatewayServiceBootRecoveryUnresolved, GatewayServiceCleanup,
    GatewayServiceCleanupDriverOutcome, GatewayServiceInstanceLease, GatewayServiceInstancePage,
    GatewayServiceInstanceState, GatewayServiceOwnershipError, Instant,
    MAX_SERVICE_BOOT_RECOVERY_CLEANUPS, MAX_SERVICE_INSTANCE_PAGE_SIZE,
};
use std::sync::Arc;
use tokio::time;

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

    pub(super) fn finish_claim(
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

    pub(super) fn finish_cleanup(&mut self, completion: CleanupCompletion) {
        if !matches!(
            completion.result,
            Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
        ) {
            self.retained.push(completion.state);
        }
    }
}

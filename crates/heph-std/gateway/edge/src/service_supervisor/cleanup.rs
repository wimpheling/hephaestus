use super::cleanup_driver::{run_cleanup_retry, run_cleanup_retry_with_recovery};
use super::{
    Arc, CleanupRetryState, GatewayServiceCleanup, GatewayServiceExpiredClaimRecovery,
    GatewayServiceSupervisor, GatewayServiceSupervisorError, GatewayServiceSupervisorJobStatus,
    Instant, Uuid,
};

impl GatewayServiceSupervisor {
    /// Schedules one parent-owned retry for terminal coordinator cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is absent, has no retained terminal
    /// cleanup responsibility, or already has a retry in flight.
    pub fn retry_cleanup(&mut self, job_id: Uuid) -> Result<(), GatewayServiceSupervisorError> {
        let (state, deadline) = self.begin_cleanup_retry(job_id)?;
        self.cleanup_jobs.push(Box::pin(run_cleanup_retry(
            job_id,
            state,
            deadline,
            Arc::clone(&self.context),
        )));
        Ok(())
    }

    /// Schedules cleanup retry with exact expired-claim recovery.
    ///
    /// This keeps the original cleanup state and capacity reservation owned by
    /// the supervisor. An expired lease may be replaced only by the supplied
    /// serialized recovery port; an ambiguous takeover is resolved through
    /// that same exact-instance barrier before physical work resumes.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is absent, has no retained terminal
    /// cleanup responsibility, or already has a retry in flight.
    pub fn retry_cleanup_with_recovery(
        &mut self,
        job_id: Uuid,
        recovery: Arc<dyn GatewayServiceExpiredClaimRecovery>,
    ) -> Result<(), GatewayServiceSupervisorError> {
        let (state, deadline) = self.begin_cleanup_retry(job_id)?;
        self.cleanup_jobs
            .push(Box::pin(run_cleanup_retry_with_recovery(
                job_id,
                state,
                deadline,
                Arc::clone(&self.context),
                recovery,
            )));
        Ok(())
    }

    fn begin_cleanup_retry(
        &mut self,
        job_id: Uuid,
    ) -> Result<(CleanupRetryState, Instant), GatewayServiceSupervisorError> {
        let started = Instant::now();
        let deadline = started
            .checked_add(self.context.policy.lease.lease_duration)
            .ok_or(GatewayServiceSupervisorError::InvalidInput)?;
        let record = self
            .records
            .get_mut(&job_id)
            .ok_or(GatewayServiceSupervisorError::RetryNotFound)?;
        if record.cleanup_in_flight {
            return Err(GatewayServiceSupervisorError::RetryAlreadyInFlight);
        }
        let failure = record
            .completion
            .as_ref()
            .and_then(|terminal| terminal.coordinator_failure.as_ref())
            .ok_or(GatewayServiceSupervisorError::RetryNotEligible)?;
        if !record.capacity_retained {
            return Err(GatewayServiceSupervisorError::RetryNotEligible);
        }
        let state = if let Some(state) = record.cleanup_retry.take() {
            state
        } else {
            let cleanup = if failure.physical_cleanup_complete {
                GatewayServiceCleanup::from_confirmed_physical(
                    failure.identity,
                    self.context.policy.instance.shutdown_timeout,
                )
            } else {
                // A missing VM handle is not proof that provider teardown
                // completed. The first retry conservatively confirms the
                // deterministic orphan before materializer cleanup.
                GatewayServiceCleanup::from_progress(
                    failure.identity,
                    failure.vm.clone(),
                    false,
                    false,
                    self.context.policy.instance.shutdown_timeout,
                )
            }
            .map_err(|_| GatewayServiceSupervisorError::InvalidInput)?;
            CleanupRetryState {
                cleanup,
                lease: failure.lease.clone(),
                pending_failure: failure.pending_failure,
                reason: Some(failure.reason),
            }
        };
        record.cleanup_in_flight = true;
        let _ = record
            .status
            .send(GatewayServiceSupervisorJobStatus::CleanupPending);
        Ok((state, deadline))
    }
}

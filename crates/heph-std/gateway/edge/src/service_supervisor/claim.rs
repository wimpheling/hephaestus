use super::claim_resolution::run_claim_resolution;
use super::cleanup_driver::run_cleanup_retry;
use super::job::complete_capacity;
use super::{
    Arc, ClaimResolutionCompletion, ClaimResolutionResult, GatewayServiceClaimResolutionStore,
    GatewayServiceSupervisor, GatewayServiceSupervisorError, GatewayServiceSupervisorEvent,
    GatewayServiceSupervisorJobStatus, Instant, Uuid, failure_from_retry,
};

impl GatewayServiceSupervisor {
    /// Resolves a late or ambiguous claim through the serialized gateway
    /// barrier and, when it is still owned by this daemon, queues cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is absent, not claim-reconciliation
    /// eligible, or already being reconciled.
    pub fn reconcile_claim(
        &mut self,
        job_id: Uuid,
        resolver: Arc<dyn GatewayServiceClaimResolutionStore>,
    ) -> Result<(), GatewayServiceSupervisorError> {
        let record = self
            .records
            .get_mut(&job_id)
            .ok_or(GatewayServiceSupervisorError::RetryNotFound)?;
        if record.claim_resolution_in_flight
            || record.cleanup_in_flight
            || record.cleanup_retry.is_some()
        {
            return Err(GatewayServiceSupervisorError::RetryAlreadyInFlight);
        }
        let terminal = record
            .completion
            .as_ref()
            .ok_or(GatewayServiceSupervisorError::RetryNotEligible)?;
        let eligible = terminal.coordinator_failure.is_none()
            && (terminal.claim_uncertain
                || (terminal.lease.is_some() && terminal.claim_cleanup_reason.is_some()));
        if !eligible || !record.capacity_retained {
            return Err(GatewayServiceSupervisorError::RetryNotEligible);
        }
        let known_lease = terminal.lease.clone();
        let reason = terminal.claim_cleanup_reason;
        let request = record.request;
        let started = Instant::now();
        let deadline = started
            .checked_add(self.context.policy.instance.probe_timeout)
            .ok_or(GatewayServiceSupervisorError::InvalidInput)?;
        record.claim_resolution_in_flight = true;
        let _ = record
            .status
            .send(GatewayServiceSupervisorJobStatus::CleanupPending);
        self.claim_resolution_jobs
            .push(Box::pin(run_claim_resolution(
                job_id,
                request,
                known_lease,
                reason,
                resolver,
                deadline,
                Arc::clone(&self.context),
            )));
        Ok(())
    }

    pub(super) fn finish_claim_resolution(
        &mut self,
        completion: ClaimResolutionCompletion,
    ) -> Option<GatewayServiceSupervisorEvent> {
        let record = self.records.get_mut(&completion.id)?;
        record.claim_resolution_in_flight = false;
        let Ok(result) = completion.result else {
            let _ = record
                .status
                .send(GatewayServiceSupervisorJobStatus::Uncertain);
            return Some(GatewayServiceSupervisorEvent {
                job_id: completion.id,
                status: GatewayServiceSupervisorJobStatus::Uncertain,
                capacity_released: false,
            });
        };
        match result {
            ClaimResolutionResult::Absent => {
                let released = complete_capacity(&self.capacity, record.token);
                record.capacity_retained = !released;
                if released {
                    let _ = record
                        .status
                        .send(GatewayServiceSupervisorJobStatus::Settled);
                    self.records.remove(&completion.id);
                } else {
                    let _ = record
                        .status
                        .send(GatewayServiceSupervisorJobStatus::Uncertain);
                }
                Some(GatewayServiceSupervisorEvent {
                    job_id: completion.id,
                    status: if released {
                        GatewayServiceSupervisorJobStatus::Settled
                    } else {
                        GatewayServiceSupervisorJobStatus::Uncertain
                    },
                    capacity_released: released,
                })
            }
            ClaimResolutionResult::Retained(_observed_lease) => {
                let _ = record
                    .status
                    .send(GatewayServiceSupervisorJobStatus::Uncertain);
                Some(GatewayServiceSupervisorEvent {
                    job_id: completion.id,
                    status: GatewayServiceSupervisorJobStatus::Uncertain,
                    capacity_released: false,
                })
            }
            ClaimResolutionResult::Owned(state) => {
                let deadline = Instant::now().checked_add(self.context.policy.lease.lease_duration);
                let Some(deadline) = deadline else {
                    record.cleanup_retry = Some(state);
                    let _ = record
                        .status
                        .send(GatewayServiceSupervisorJobStatus::Uncertain);
                    return Some(GatewayServiceSupervisorEvent {
                        job_id: completion.id,
                        status: GatewayServiceSupervisorJobStatus::Uncertain,
                        capacity_released: false,
                    });
                };
                if let Some(terminal) = record.completion.as_mut() {
                    terminal.claim_uncertain = false;
                    terminal.lease = Some(state.lease.clone());
                    terminal.coordinator_failure = Some(failure_from_retry(&state));
                }
                record.cleanup_in_flight = true;
                record.cleanup_retry = Some(state);
                self.cleanup_jobs.push(Box::pin(run_cleanup_retry(
                    completion.id,
                    record.cleanup_retry.take().expect("claim cleanup state"),
                    deadline,
                    Arc::clone(&self.context),
                )));
                let _ = record
                    .status
                    .send(GatewayServiceSupervisorJobStatus::CleanupPending);
                Some(GatewayServiceSupervisorEvent {
                    job_id: completion.id,
                    status: GatewayServiceSupervisorJobStatus::CleanupPending,
                    capacity_released: false,
                })
            }
        }
    }
}

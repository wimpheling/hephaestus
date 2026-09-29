use super::job::complete_capacity;
use super::poll_futures::{poll_next_claim_resolution, poll_next_cleanup, poll_next_job};
use super::{
    CleanupCompletion, GatewayServiceSupervisor, GatewayServiceSupervisorEvent,
    GatewayServiceSupervisorJobStatus, SupervisorCompletion, failure_from_retry,
};
impl GatewayServiceSupervisor {
    /// Waits for one job completion. Dropping this wait does not drop jobs.
    /// The caller must continue polling and eventually call [`Self::shutdown`]
    /// to settle retained cleanup responsibility before dropping the supervisor.
    ///
    /// # Panics
    ///
    /// Panics only if an internal job record is missing or the private
    /// capacity mutex was poisoned by a previous panic in this process.
    pub async fn poll(&mut self) -> Option<GatewayServiceSupervisorEvent> {
        if self.jobs.is_empty()
            && self.cleanup_jobs.is_empty()
            && self.claim_resolution_jobs.is_empty()
        {
            return None;
        }
        let completion = tokio::select! {
            completion = poll_next_job(&mut self.jobs), if !self.jobs.is_empty() => SupervisorCompletion::Startup(completion?),
            completion = poll_next_cleanup(&mut self.cleanup_jobs), if !self.cleanup_jobs.is_empty() => SupervisorCompletion::Cleanup(completion?),
            completion = poll_next_claim_resolution(&mut self.claim_resolution_jobs), if !self.claim_resolution_jobs.is_empty() => SupervisorCompletion::ClaimResolution(completion?),
        };
        if let SupervisorCompletion::ClaimResolution(completion) = completion {
            return self.finish_claim_resolution(completion);
        }
        if let SupervisorCompletion::Cleanup(completion) = completion {
            return self.finish_cleanup(completion);
        }
        let SupervisorCompletion::Startup(completion) = completion else {
            unreachable!("cleanup completion returned above")
        };
        let record = self
            .records
            .get_mut(&completion.id)
            .expect("startup job record");
        record.capacity_retained = !completion.capacity_released;
        record.completion = Some(completion.terminal);
        let status = completion.status;
        record.status.send_replace(status);
        let event = GatewayServiceSupervisorEvent {
            job_id: completion.id,
            status,
            capacity_released: completion.capacity_released,
        };
        if completion.capacity_released {
            self.records.remove(&completion.id);
        }
        Some(event)
    }

    fn finish_cleanup(
        &mut self,
        completion: CleanupCompletion,
    ) -> Option<GatewayServiceSupervisorEvent> {
        let record = self.records.get_mut(&completion.id)?;
        record.cleanup_in_flight = false;
        if completion.result.is_ok() {
            let released = complete_capacity(&self.capacity, record.token);
            record.capacity_retained = !released;
            if released {
                record.cleanup_retry = None;
                record.completion = None;
                let _ = record
                    .status
                    .send(GatewayServiceSupervisorJobStatus::Settled);
                self.records.remove(&completion.id);
                Some(GatewayServiceSupervisorEvent {
                    job_id: completion.id,
                    status: GatewayServiceSupervisorJobStatus::Settled,
                    capacity_released: true,
                })
            } else {
                record.cleanup_retry = Some(completion.state);
                let _ = record
                    .status
                    .send(GatewayServiceSupervisorJobStatus::Failed);
                Some(GatewayServiceSupervisorEvent {
                    job_id: completion.id,
                    status: GatewayServiceSupervisorJobStatus::Failed,
                    capacity_released: false,
                })
            }
        } else {
            let failure = failure_from_retry(&completion.state);
            if let Some(terminal) = record.completion.as_mut() {
                terminal.lease = Some(completion.state.lease.clone());
                terminal.coordinator_failure = Some(failure);
            }
            record.cleanup_retry = Some(completion.state);
            let _ = record
                .status
                .send(GatewayServiceSupervisorJobStatus::Failed);
            Some(GatewayServiceSupervisorEvent {
                job_id: completion.id,
                status: GatewayServiceSupervisorJobStatus::Failed,
                capacity_released: false,
            })
        }
    }
}

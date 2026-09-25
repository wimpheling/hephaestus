use super::{
    GatewayServiceSupervisor, GatewayServiceSupervisorShutdown, GatewayServiceSupervisorUnresolved,
};
impl GatewayServiceSupervisor {
    /// Returns the current reservation counts without releasing any job.
    ///
    /// # Panics
    ///
    /// Panics only if the private capacity mutex was poisoned by a previous
    /// panic in this process.
    #[must_use]
    pub fn capacity_snapshot(&self) -> crate::GatewayServiceCapacitySnapshot {
        self.capacity
            .lock()
            .expect("service capacity lock")
            .snapshot()
    }

    /// Returns whether [`Self::poll`] has a queued or running future to await.
    ///
    /// A false result can still accompany retained unresolved capacity; those
    /// records require later reconciliation rather than a busy polling loop.
    #[must_use]
    pub fn has_pending_jobs(&self) -> bool {
        !self.jobs.is_empty()
            || !self.cleanup_jobs.is_empty()
            || !self.claim_resolution_jobs.is_empty()
    }

    /// Consumes the supervisor, cancels every job, and joins all owned futures.
    pub async fn shutdown(mut self) -> GatewayServiceSupervisorShutdown {
        for record in self.records.values() {
            record.cancellation.cancel();
        }
        while self.poll().await.is_some() {}
        let unresolved = self
            .records
            .into_iter()
            .filter_map(|(job_id, record)| {
                record.completion.and_then(|completion| {
                    if record.capacity_retained {
                        let (cleanup, pending_failure, retry_reason) =
                            record.cleanup_retry.map_or((None, None, None), |retry| {
                                (Some(retry.cleanup), retry.pending_failure, retry.reason)
                            });
                        let original_reason = retry_reason.or(completion.claim_cleanup_reason);
                        Some(GatewayServiceSupervisorUnresolved {
                            job_id,
                            request: record.request,
                            capacity_token: record.token,
                            lease: completion.lease,
                            claim_uncertain: completion.claim_uncertain,
                            coordinator_failure: completion.coordinator_failure,
                            cleanup,
                            pending_failure,
                            original_reason,
                        })
                    } else {
                        None
                    }
                })
            })
            .collect();
        GatewayServiceSupervisorShutdown { unresolved }
    }
}

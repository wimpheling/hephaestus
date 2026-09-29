use super::job::run_job;
use super::{
    Arc, CancellationToken, GatewayServiceCapacity, GatewayServiceCapacityError,
    GatewayServiceLogWriterConfig, GatewayServiceStartupHandle, GatewayServiceStartupRequest,
    GatewayServiceSupervisor, GatewayServiceSupervisorContext, GatewayServiceSupervisorError,
    GatewayServiceSupervisorJobStatus, HashMap, Instant, JobRecord, Mutex, StartupDeadlines, Uuid,
    watch,
};
impl GatewayServiceSupervisor {
    /// Creates a supervisor after validating all post-claim constructor inputs.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceSupervisorError::InvalidInput`] for an invalid
    /// owner, policy, or HTTP authority.
    pub fn new(
        context: GatewayServiceSupervisorContext,
    ) -> Result<Self, GatewayServiceSupervisorError> {
        context
            .owner
            .validate()
            .map_err(|_| GatewayServiceSupervisorError::InvalidInput)?;
        context
            .policy
            .validate()
            .map_err(|_| GatewayServiceSupervisorError::InvalidInput)?;
        if context.service_authority.is_empty()
            || http::uri::Authority::try_from(context.service_authority.as_str()).is_err()
        {
            return Err(GatewayServiceSupervisorError::InvalidInput);
        }
        let capacity = GatewayServiceCapacity::new(context.policy)
            .map_err(GatewayServiceSupervisorError::Capacity)?;
        Ok(Self {
            context: Arc::new(context),
            log_writer: None,
            capacity: Arc::new(Mutex::new(capacity)),
            jobs: Vec::new(),
            cleanup_jobs: Vec::new(),
            claim_resolution_jobs: Vec::new(),
            records: HashMap::new(),
        })
    }

    /// Attaches an optional worker-owned durable application-log writer.
    #[must_use]
    pub fn with_log_writer(mut self, config: GatewayServiceLogWriterConfig) -> Self {
        self.log_writer = Some(config);
        self
    }

    /// Starts one bounded job after reserving capacity before claim.
    ///
    /// # Errors
    ///
    /// Returns a capacity or duplicate error before durable ownership is
    /// touched.
    ///
    /// # Panics
    ///
    /// Panics only if the private capacity mutex was poisoned by a previous
    /// panic in this process.
    pub fn start(
        &mut self,
        request: GatewayServiceStartupRequest,
    ) -> Result<GatewayServiceStartupHandle, GatewayServiceSupervisorError> {
        if request.gateway_id.is_nil() || request.revision_id.is_nil() {
            return Err(GatewayServiceSupervisorError::InvalidInput);
        }
        if self.records.values().any(|record| {
            record.request.gateway_id == request.gateway_id
                && record.request.revision_id == request.revision_id
                && record.capacity_retained
        }) {
            return Err(GatewayServiceSupervisorError::Duplicate);
        }
        let claim_started = Instant::now();
        let Some(initial_lease_deadline) =
            claim_started.checked_add(self.context.policy.lease.lease_duration)
        else {
            return Err(GatewayServiceSupervisorError::InvalidInput);
        };
        let Some(startup_deadline) =
            claim_started.checked_add(self.context.policy.instance.startup_timeout)
        else {
            return Err(GatewayServiceSupervisorError::InvalidInput);
        };
        let token = self
            .capacity
            .lock()
            .expect("service capacity lock")
            .reserve(request.gateway_id, request.revision_id)
            .map_err(|error| match error {
                GatewayServiceCapacityError::DuplicateRevision => {
                    GatewayServiceSupervisorError::Duplicate
                }
                other => GatewayServiceSupervisorError::Capacity(other),
            })?;
        let id = Uuid::new_v4();
        let cancellation = CancellationToken::new();
        let (drain, drain_receiver) = watch::channel(false);
        let (status, status_receiver) = watch::channel(GatewayServiceSupervisorJobStatus::Claiming);
        self.records.insert(
            id,
            JobRecord {
                request,
                token,
                cancellation: cancellation.clone(),
                status: status.clone(),
                capacity_retained: true,
                completion: None,
                cleanup_retry: None,
                cleanup_in_flight: false,
                claim_resolution_in_flight: false,
            },
        );
        self.jobs.push(Box::pin(run_job(
            id,
            request,
            token,
            StartupDeadlines {
                initial_lease_deadline,
                startup_deadline,
            },
            Arc::clone(&self.context),
            self.log_writer.clone(),
            Arc::clone(&self.capacity),
            cancellation.clone(),
            status,
            drain_receiver,
        )));
        Ok(GatewayServiceStartupHandle {
            id,
            cancellation,
            drain,
            status: status_receiver,
        })
    }
}

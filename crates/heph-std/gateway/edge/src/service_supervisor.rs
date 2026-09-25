//! Parent-owned concurrent startup bookkeeping for persistent services.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Poll,
};

use tokio::{
    sync::watch,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vm_trait::VmProvider;

use crate::{
    GatewayServiceCapacity, GatewayServiceCapacityError, GatewayServiceCapacityToken,
    GatewayServiceClaimResolutionStore, GatewayServiceCleanup, GatewayServiceCleanupDriver,
    GatewayServiceCleanupDriverError, GatewayServiceCleanupDriverOutcome,
    GatewayServiceCleanupDriverPolicy, GatewayServiceCoordinator, GatewayServiceCoordinatorFailure,
    GatewayServiceCoordinatorFailureReason, GatewayServiceCoordinatorStatus,
    GatewayServiceExpiredClaimRecovery, GatewayServiceFailure, GatewayServiceFailureStore,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceLaunchResolver,
    GatewayServiceLogWriterConfig, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError, GatewayServiceRegistry, GatewayServiceStartupIntent,
    GatewayServiceSupervisorPolicy, GatewayServiceTargetStore,
};

/// Validated immutable dependencies shared by all startup jobs.
pub struct GatewayServiceSupervisorContext {
    /// Daemon owner identity used for every claim and coordinator.
    pub owner: GatewayServiceOwner,
    /// Bounded service lifecycle policy.
    pub policy: GatewayServiceSupervisorPolicy,
    /// Durable ownership adapter.
    pub ownership: Arc<dyn GatewayServiceOwnership>,
    /// Durable redacted failure adapter.
    pub failure_store: Arc<dyn GatewayServiceFailureStore>,
    /// Immutable service launch resolver.
    pub resolver: Arc<dyn GatewayServiceLaunchResolver>,
    /// VM provider used by coordinators.
    pub provider: Arc<dyn VmProvider>,
    /// Exact target lookup adapter used by coordinators.
    pub targets: Arc<dyn GatewayServiceTargetStore>,
    /// Shared ready-instance registry.
    pub registry: GatewayServiceRegistry,
    /// Host-owned authority used for readiness probes.
    pub service_authority: String,
}

/// A startup request selected by an already-running reconciliation caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceStartupRequest {
    /// Gateway whose service revision is being started.
    pub gateway_id: Uuid,
    /// Exact immutable service revision.
    pub revision_id: Uuid,
    /// Whether readiness should activate or restore this revision.
    pub intent: GatewayServiceStartupIntent,
}

/// Public lifecycle state for one parent-owned startup job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceSupervisorJobStatus {
    /// Capacity is reserved and the durable claim is in flight.
    Claiming,
    /// A claim exists and coordinator preparation is in flight.
    Starting,
    /// The coordinator reached readiness and startup capacity was released.
    Ready,
    /// The job ended with all physical and durable cleanup complete.
    Settled,
    /// The job ended while retaining an exact claim or cleanup responsibility.
    Failed,
    /// The claim operation may have committed but did not return a result.
    Uncertain,
    /// The caller requested cancellation and the job retained cleanup state.
    Cancelled,
    /// A parent-owned cleanup retry is currently running.
    CleanupPending,
}

/// A caller-owned handle for cancellation and status observation.
#[derive(Debug)]
pub struct GatewayServiceStartupHandle {
    id: Uuid,
    cancellation: CancellationToken,
    drain: watch::Sender<bool>,
    status: watch::Receiver<GatewayServiceSupervisorJobStatus>,
}

impl GatewayServiceStartupHandle {
    /// Returns the stable job identifier.
    #[must_use]
    pub const fn job_id(&self) -> Uuid {
        self.id
    }

    /// Requests cancellation while preserving the supervisor-owned job.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Requests graceful retirement of the service after accepted calls drain.
    pub fn request_drain(&self) {
        self.drain.send_replace(true);
    }

    /// Subscribes to lifecycle status changes.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<GatewayServiceSupervisorJobStatus> {
        self.status.clone()
    }
}

/// Completion notification returned by [`GatewayServiceSupervisor::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceSupervisorEvent {
    /// Completed job identifier.
    pub job_id: Uuid,
    /// Terminal public status.
    pub status: GatewayServiceSupervisorJobStatus,
    /// Whether the capacity reservation has been released.
    pub capacity_released: bool,
}

/// Exact unresolved state returned when the supervisor is consumed at shutdown.
pub struct GatewayServiceSupervisorUnresolved {
    /// Job that still owns this state.
    pub job_id: Uuid,
    /// Exact gateway/revision request needed by later reconciliation.
    pub request: GatewayServiceStartupRequest,
    /// Exact capacity reservation retained for later reconciliation.
    pub capacity_token: GatewayServiceCapacityToken,
    /// Late durable claim, when one was returned.
    pub lease: Option<GatewayServiceInstanceLease>,
    /// Whether the claim operation ended without an authoritative result.
    pub claim_uncertain: bool,
    /// Coordinator failure retaining any VM or materialization responsibility.
    pub coordinator_failure: Option<GatewayServiceCoordinatorFailure>,
    /// Physical and materializer cleanup progress retained for retry.
    pub cleanup: Option<GatewayServiceCleanup>,
    /// Failure report still awaiting durable recording.
    pub pending_failure: Option<GatewayServiceFailure>,
    /// Original coordinator termination reason.
    pub original_reason: Option<GatewayServiceCoordinatorFailureReason>,
}

/// Result of consuming a supervisor after cancellation and job settlement.
pub struct GatewayServiceSupervisorShutdown {
    /// Jobs whose capacity and cleanup responsibility remain unresolved.
    pub unresolved: Vec<GatewayServiceSupervisorUnresolved>,
}

/// Construction and start failures which do not transfer ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceSupervisorError {
    /// Context, request, or deadline input was malformed.
    #[error("invalid gateway service supervisor input")]
    InvalidInput,
    /// Capacity was unavailable before any durable claim was attempted.
    #[error("gateway service startup capacity is unavailable")]
    Capacity(#[from] GatewayServiceCapacityError),
    /// A job for this exact gateway and revision already exists.
    #[error("gateway service startup is already reserved")]
    Duplicate,
    /// No terminal coordinator cleanup exists for this job.
    #[error("gateway service cleanup retry is not eligible")]
    RetryNotEligible,
    /// The requested cleanup job no longer exists.
    #[error("gateway service cleanup retry job was not found")]
    RetryNotFound,
    /// A cleanup retry is already parent-owned and running.
    #[error("gateway service cleanup retry is already in flight")]
    RetryAlreadyInFlight,
    /// The claim-resolution store could not complete its bounded read.
    #[error("gateway service claim resolution is unavailable")]
    ClaimResolutionUnavailable,
}

/// Parent-owned bounded set of concurrent startup jobs.
pub struct GatewayServiceSupervisor {
    context: Arc<GatewayServiceSupervisorContext>,
    log_writer: Option<GatewayServiceLogWriterConfig>,
    capacity: Arc<Mutex<GatewayServiceCapacity>>,
    jobs: Vec<Pin<Box<dyn Future<Output = JobCompletion> + Send>>>,
    cleanup_jobs: Vec<Pin<Box<dyn Future<Output = CleanupCompletion> + Send>>>,
    claim_resolution_jobs: Vec<Pin<Box<dyn Future<Output = ClaimResolutionCompletion> + Send>>>,
    records: HashMap<Uuid, JobRecord>,
}

#[derive(Clone, Copy)]
struct StartupDeadlines {
    initial_lease_deadline: Instant,
    startup_deadline: Instant,
}

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

    fn finish_claim_resolution(
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

fn failure_from_retry(state: &CleanupRetryState) -> GatewayServiceCoordinatorFailure {
    GatewayServiceCoordinatorFailure {
        identity: state.cleanup.identity(),
        lease: state.lease.clone(),
        reason: state
            .reason
            .unwrap_or(GatewayServiceCoordinatorFailureReason::Ownership),
        vm: state.cleanup.retained_vm(),
        materialization_owned: !state.cleanup.materializer_cleanup_confirmed(),
        physical_cleanup_complete: state.cleanup.vm_teardown_confirmed()
            && state.cleanup.materializer_cleanup_confirmed(),
        durable_cleanup_complete: false,
        pending_failure: state.pending_failure,
    }
}

struct JobRecord {
    request: GatewayServiceStartupRequest,
    token: GatewayServiceCapacityToken,
    cancellation: CancellationToken,
    status: watch::Sender<GatewayServiceSupervisorJobStatus>,
    capacity_retained: bool,
    completion: Option<JobTerminal>,
    cleanup_retry: Option<CleanupRetryState>,
    cleanup_in_flight: bool,
    claim_resolution_in_flight: bool,
}

struct JobCompletion {
    id: Uuid,
    status: GatewayServiceSupervisorJobStatus,
    capacity_released: bool,
    terminal: JobTerminal,
}

struct JobTerminal {
    lease: Option<GatewayServiceInstanceLease>,
    claim_uncertain: bool,
    coordinator_failure: Option<GatewayServiceCoordinatorFailure>,
    claim_cleanup_reason: Option<GatewayServiceCoordinatorFailureReason>,
}

struct CleanupRetryState {
    cleanup: GatewayServiceCleanup,
    lease: GatewayServiceInstanceLease,
    pending_failure: Option<GatewayServiceFailure>,
    reason: Option<GatewayServiceCoordinatorFailureReason>,
}

struct CleanupCompletion {
    id: Uuid,
    state: CleanupRetryState,
    result: Result<(), GatewayServiceCleanupDriverError>,
}

enum ClaimResolutionResult {
    Absent,
    Owned(CleanupRetryState),
    Retained(Option<GatewayServiceInstanceLease>),
}

struct ClaimResolutionCompletion {
    id: Uuid,
    result: Result<ClaimResolutionResult, GatewayServiceSupervisorError>,
}

enum SupervisorCompletion {
    Startup(JobCompletion),
    Cleanup(CleanupCompletion),
    ClaimResolution(ClaimResolutionCompletion),
}

async fn poll_next_job(
    jobs: &mut Vec<Pin<Box<dyn Future<Output = JobCompletion> + Send>>>,
) -> Option<JobCompletion> {
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

async fn poll_next_cleanup(
    jobs: &mut Vec<Pin<Box<dyn Future<Output = CleanupCompletion> + Send>>>,
) -> Option<CleanupCompletion> {
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

async fn poll_next_claim_resolution(
    jobs: &mut Vec<Pin<Box<dyn Future<Output = ClaimResolutionCompletion> + Send>>>,
) -> Option<ClaimResolutionCompletion> {
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

async fn run_cleanup_retry(
    id: Uuid,
    mut state: CleanupRetryState,
    renew_deadline: Instant,
    context: Arc<GatewayServiceSupervisorContext>,
) -> CleanupCompletion {
    let policy = GatewayServiceCleanupDriverPolicy {
        lease: context.policy.lease,
        database_timeout: context.policy.instance.probe_timeout,
    };
    let result = match GatewayServiceCleanupDriver::new(
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.targets),
        Arc::clone(&context.provider),
        Arc::clone(&context.resolver),
        context.owner.clone(),
        policy,
    ) {
        Ok(driver) => match driver
            .confirm_cleaned_state(&state.cleanup, &state.lease)
            .await
        {
            Ok(true) => Ok(()),
            Ok(false) | Err(GatewayServiceCleanupDriverError::Unavailable) => {
                match driver
                    .renew_and_prepare_stopping(&mut state.lease, renew_deadline)
                    .await
                {
                    Ok(cleanup_deadline) => {
                        match driver
                            .attempt(
                                &mut state.cleanup,
                                &mut state.lease,
                                &mut state.pending_failure,
                                cleanup_deadline,
                            )
                            .await
                        {
                            Ok(GatewayServiceCleanupDriverOutcome::Cleaned) => Ok(()),
                            Ok(GatewayServiceCleanupDriverOutcome::Pending { .. }) => {
                                Err(GatewayServiceCleanupDriverError::Unavailable)
                            }
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    CleanupCompletion { id, state, result }
}

async fn run_cleanup_retry_with_recovery(
    id: Uuid,
    mut state: CleanupRetryState,
    deadline: Instant,
    context: Arc<GatewayServiceSupervisorContext>,
    recovery: Arc<dyn GatewayServiceExpiredClaimRecovery>,
) -> CleanupCompletion {
    let policy = GatewayServiceCleanupDriverPolicy {
        lease: context.policy.lease,
        database_timeout: context.policy.instance.probe_timeout,
    };
    let result = match GatewayServiceCleanupDriver::new(
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.targets),
        Arc::clone(&context.provider),
        Arc::clone(&context.resolver),
        context.owner.clone(),
        policy,
    ) {
        Ok(driver) => {
            let confirmation = driver
                .confirm_cleaned_state(&state.cleanup, &state.lease)
                .await;
            match confirmation {
                Ok(true) => Ok(()),
                Ok(false) | Err(GatewayServiceCleanupDriverError::Unavailable) => {
                    renew_or_recover_cleanup(&driver, &mut state, deadline, &context, &recovery)
                        .await
                }
                Err(GatewayServiceCleanupDriverError::Stale) => {
                    recover_and_cleanup(&driver, &mut state, deadline, &context, &recovery).await
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    CleanupCompletion { id, state, result }
}

async fn renew_or_recover_cleanup(
    driver: &GatewayServiceCleanupDriver,
    state: &mut CleanupRetryState,
    deadline: Instant,
    context: &GatewayServiceSupervisorContext,
    recovery: &Arc<dyn GatewayServiceExpiredClaimRecovery>,
) -> Result<(), GatewayServiceCleanupDriverError> {
    match driver
        .renew_and_prepare_stopping(&mut state.lease, deadline)
        .await
    {
        Ok(cleanup_deadline) => run_cleanup_attempt(driver, state, cleanup_deadline).await,
        Err(GatewayServiceCleanupDriverError::Stale) => {
            recover_and_cleanup(driver, state, deadline, context, recovery).await
        }
        Err(error) => Err(error),
    }
}

async fn recover_and_cleanup(
    driver: &GatewayServiceCleanupDriver,
    state: &mut CleanupRetryState,
    deadline: Instant,
    context: &GatewayServiceSupervisorContext,
    recovery: &Arc<dyn GatewayServiceExpiredClaimRecovery>,
) -> Result<(), GatewayServiceCleanupDriverError> {
    let (candidate, recovery_started) = recover_expired_claim(
        recovery,
        &state.lease,
        &context.owner,
        context.policy.lease.lease_duration,
        context.policy.instance.probe_timeout,
        deadline,
        state.cleanup.vm_teardown_confirmed() && state.cleanup.materializer_cleanup_confirmed(),
    )
    .await?;
    // Adopt only after all identity, owner, VM, fence, and state checks pass.
    // A validated successor remains caller-owned even when its renewal fails.
    state.lease = candidate;
    if state.lease.state == GatewayServiceInstanceState::Cleaned {
        if state.cleanup.vm_teardown_confirmed()
            && state.cleanup.materializer_cleanup_confirmed()
            && driver
                .confirm_cleaned_state(&state.cleanup, &state.lease)
                .await?
        {
            return Ok(());
        }
        return Err(GatewayServiceCleanupDriverError::Unavailable);
    }
    let renewal_deadline = recovery_started
        .checked_add(context.policy.lease.lease_duration)
        .map_or(deadline, |candidate| candidate.min(deadline));
    // The observed successor grants no physical authority until this exact
    // lease is renewed under a fresh conservative deadline.
    let cleanup_deadline = driver
        .renew_and_prepare_stopping(&mut state.lease, renewal_deadline)
        .await?;
    run_cleanup_attempt(driver, state, cleanup_deadline).await
}

async fn run_cleanup_attempt(
    driver: &GatewayServiceCleanupDriver,
    state: &mut CleanupRetryState,
    deadline: Instant,
) -> Result<(), GatewayServiceCleanupDriverError> {
    match driver
        .attempt(
            &mut state.cleanup,
            &mut state.lease,
            &mut state.pending_failure,
            deadline,
        )
        .await?
    {
        GatewayServiceCleanupDriverOutcome::Cleaned => Ok(()),
        GatewayServiceCleanupDriverOutcome::Pending { .. } => {
            Err(GatewayServiceCleanupDriverError::Unavailable)
        }
    }
}

async fn recover_expired_claim(
    recovery: &Arc<dyn GatewayServiceExpiredClaimRecovery>,
    previous: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    lease_duration: std::time::Duration,
    database_timeout: std::time::Duration,
    overall_deadline: Instant,
    physical_confirmed: bool,
) -> Result<(GatewayServiceInstanceLease, Instant), GatewayServiceCleanupDriverError> {
    if !valid_claim_identity(previous) {
        return Err(GatewayServiceCleanupDriverError::Stale);
    }
    if overall_deadline <= Instant::now() {
        return Err(GatewayServiceCleanupDriverError::Stale);
    }
    let call_started = Instant::now();
    let database_deadline = call_started
        .checked_add(database_timeout)
        .ok_or(GatewayServiceCleanupDriverError::InvalidInput)?;
    let call_deadline = database_deadline.min(overall_deadline);
    let takeover = time::timeout_at(
        call_deadline,
        recovery.claim_expired_instance(previous, owner, lease_duration),
    )
    .await;
    let candidate = match takeover {
        Ok(Ok(candidate)) => candidate,
        Ok(Err(
            GatewayServiceOwnershipError::StaleLease | GatewayServiceOwnershipError::Unavailable,
        ))
        | Err(_) => {
            let resolve_started = Instant::now();
            let resolve_deadline = resolve_started
                .checked_add(database_timeout)
                .ok_or(GatewayServiceCleanupDriverError::InvalidInput)?
                .min(overall_deadline);
            time::timeout_at(
                resolve_deadline,
                recovery.resolve_exact_instance(previous.identity),
            )
            .await
            .map_err(|_| GatewayServiceCleanupDriverError::Unavailable)?
            .map_err(map_recovery_ownership_error)?
            .ok_or(GatewayServiceCleanupDriverError::Unavailable)?
        }
        Ok(Err(error)) => return Err(map_recovery_ownership_error(error)),
    };
    let successor_valid =
        valid_expired_recovery_successor(previous, &candidate, owner, physical_confirmed);
    let cleaned_valid = valid_claim_identity(previous)
        && valid_claim_identity(&candidate)
        && physical_confirmed
        && candidate.state == GatewayServiceInstanceState::Cleaned
        && same_claim(&candidate, previous);
    if !(successor_valid || cleaned_valid) {
        return Err(GatewayServiceCleanupDriverError::Stale);
    }
    Ok((candidate, call_started))
}

fn valid_expired_recovery_successor(
    previous: &GatewayServiceInstanceLease,
    candidate: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    physical_confirmed: bool,
) -> bool {
    let same_host =
        candidate.owner_host_id == owner.host_id && previous.owner_host_id == owner.host_id;
    previous
        .fencing_token
        .checked_add(1)
        .is_some_and(|next_fence| {
            valid_claim_identity(previous)
                && valid_claim_identity(candidate)
                && candidate.identity == previous.identity
                && candidate.vm_id == previous.vm_id
                && same_host
                && candidate.owner_uuid == owner.owner_uuid
                && candidate.fencing_token == next_fence
                && (candidate.state == GatewayServiceInstanceState::Stopping
                    || (physical_confirmed
                        && candidate.state == GatewayServiceInstanceState::Cleaned))
                && candidate.lease_expires_at > candidate.heartbeat_at
        })
}

const fn map_recovery_ownership_error(
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

async fn run_claim_resolution(
    id: Uuid,
    request: GatewayServiceStartupRequest,
    known_lease: Option<GatewayServiceInstanceLease>,
    reason: Option<GatewayServiceCoordinatorFailureReason>,
    resolver: Arc<dyn GatewayServiceClaimResolutionStore>,
    deadline: Instant,
    context: Arc<GatewayServiceSupervisorContext>,
) -> ClaimResolutionCompletion {
    let result = match time::timeout_at(
        deadline,
        resolver.resolve_revision_claim(request.gateway_id, request.revision_id),
    )
    .await
    {
        Ok(Ok(None)) => Ok(ClaimResolutionResult::Absent),
        Ok(Ok(Some(lease))) => resolve_claim_result(request, known_lease, reason, lease, &context),
        Ok(Err(GatewayServiceOwnershipError::Unavailable)) | Err(_) => {
            Err(GatewayServiceSupervisorError::ClaimResolutionUnavailable)
        }
        Ok(Err(_)) => Ok(ClaimResolutionResult::Retained(None)),
    };
    ClaimResolutionCompletion { id, result }
}

fn resolve_claim_result(
    request: GatewayServiceStartupRequest,
    known_lease: Option<GatewayServiceInstanceLease>,
    reason: Option<GatewayServiceCoordinatorFailureReason>,
    lease: GatewayServiceInstanceLease,
    context: &GatewayServiceSupervisorContext,
) -> Result<ClaimResolutionResult, GatewayServiceSupervisorError> {
    if !valid_claim_identity(&lease)
        || lease.identity.gateway_id != request.gateway_id
        || lease.identity.revision_id != request.revision_id
        || lease.owner_host_id != context.owner.host_id
        || lease.owner_uuid != context.owner.owner_uuid
        || lease.lease_expires_at <= ::time::OffsetDateTime::now_utc()
        || !lease.state.is_live()
    {
        return Ok(ClaimResolutionResult::Retained(Some(lease)));
    }
    if let Some(known) = known_lease {
        if !same_claim(&known, &lease) {
            return Ok(ClaimResolutionResult::Retained(Some(lease)));
        }
    }
    let cleanup = GatewayServiceCleanup::new(
        lease.identity,
        None,
        context.policy.instance.shutdown_timeout,
    )
    .map_err(|_| GatewayServiceSupervisorError::InvalidInput)?;
    Ok(ClaimResolutionResult::Owned(CleanupRetryState {
        cleanup,
        lease,
        pending_failure: None,
        reason,
    }))
}

fn valid_claim_identity(lease: &GatewayServiceInstanceLease) -> bool {
    lease.identity.instance_id != Uuid::nil()
        && lease.identity.gateway_id != Uuid::nil()
        && lease.identity.revision_id != Uuid::nil()
        && lease.fencing_token > 0
        && lease.vm_id == format!("gateway-service-{}", lease.identity.instance_id)
        && lease.lease_expires_at > lease.heartbeat_at
}

fn same_claim(left: &GatewayServiceInstanceLease, right: &GatewayServiceInstanceLease) -> bool {
    left.identity == right.identity
        && left.vm_id == right.vm_id
        && left.owner_host_id == right.owner_host_id
        && left.owner_uuid == right.owner_uuid
        && left.fencing_token == right.fencing_token
}

// This function intentionally owns the complete claim/coordinator state
// machine so cancellation cannot drop a durable claim outside the supervisor.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn run_job(
    id: Uuid,
    request: GatewayServiceStartupRequest,
    token: GatewayServiceCapacityToken,
    deadlines: StartupDeadlines,
    context: Arc<GatewayServiceSupervisorContext>,
    log_writer: Option<GatewayServiceLogWriterConfig>,
    capacity: Arc<Mutex<GatewayServiceCapacity>>,
    cancellation: CancellationToken,
    status: watch::Sender<GatewayServiceSupervisorJobStatus>,
    mut drain: watch::Receiver<bool>,
) -> JobCompletion {
    if cancellation.is_cancelled() || Instant::now() >= deadlines.startup_deadline {
        let status_value = if cancellation.is_cancelled() {
            GatewayServiceSupervisorJobStatus::Cancelled
        } else {
            GatewayServiceSupervisorJobStatus::Failed
        };
        return terminal(
            id,
            token,
            &capacity,
            &status,
            status_value,
            None,
            false,
            true,
            None,
        );
    }
    let claim = context.ownership.claim_new(
        request.gateway_id,
        request.revision_id,
        &context.owner,
        context.policy.lease.lease_duration,
    );
    tokio::pin!(claim);
    let claim_result = tokio::select! {
        result = &mut claim => result,
        () = cancellation.cancelled() => {
            let _ = status.send(GatewayServiceSupervisorJobStatus::Cancelled);
            claim.await
        }
        () = time::sleep_until(deadlines.startup_deadline) => {
            let _ = status.send(GatewayServiceSupervisorJobStatus::Cancelled);
            claim.await
        }
    };
    let lease = match claim_result {
        Ok(lease) => lease,
        Err(
            GatewayServiceOwnershipError::Conflict
            | GatewayServiceOwnershipError::InvalidArgument
            | GatewayServiceOwnershipError::StaleLease,
        ) => {
            return terminal(
                id,
                token,
                &capacity,
                &status,
                GatewayServiceSupervisorJobStatus::Failed,
                None,
                false,
                true,
                None,
            );
        }
        Err(GatewayServiceOwnershipError::Unavailable) => {
            return terminal(
                id,
                token,
                &capacity,
                &status,
                GatewayServiceSupervisorJobStatus::Uncertain,
                None,
                false,
                false,
                None,
            );
        }
    };
    if lease.identity.gateway_id != request.gateway_id
        || lease.identity.revision_id != request.revision_id
    {
        return terminal(
            id,
            token,
            &capacity,
            &status,
            GatewayServiceSupervisorJobStatus::Failed,
            Some(lease),
            false,
            false,
            None,
        );
    }
    if cancellation.is_cancelled() || Instant::now() >= deadlines.startup_deadline {
        let mut completion = terminal(
            id,
            token,
            &capacity,
            &status,
            GatewayServiceSupervisorJobStatus::Cancelled,
            Some(lease),
            false,
            false,
            None,
        );
        completion.terminal.claim_cleanup_reason = Some(if cancellation.is_cancelled() {
            GatewayServiceCoordinatorFailureReason::Cancelled
        } else {
            GatewayServiceCoordinatorFailureReason::StartupDeadline
        });
        return completion;
    }
    let _ = status.send(GatewayServiceSupervisorJobStatus::Starting);
    let coordinator = GatewayServiceCoordinator::new(
        lease.clone(),
        context.owner.clone(),
        deadlines.initial_lease_deadline,
        deadlines.startup_deadline,
        request.intent,
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.resolver),
        Arc::clone(&context.provider),
        Arc::clone(&context.targets),
        context.registry.clone(),
        context.service_authority.clone(),
        context.policy,
    );
    let Ok((coordinator, control)) = coordinator else {
        return terminal(
            id,
            token,
            &capacity,
            &status,
            GatewayServiceSupervisorJobStatus::Failed,
            Some(lease),
            false,
            false,
            None,
        );
    };
    let coordinator_status = control.subscribe();
    let mut coordinator_status = coordinator_status;
    let mut run = Box::pin(
        (if let Some(config) = log_writer {
            coordinator.with_log_writer(config)
        } else {
            coordinator
        })
        .run(),
    );
    let mut cancel_sent = false;
    let mut startup_finished = false;
    let mut status_open = true;
    let mut drain_open = true;
    if *drain.borrow() {
        control.request_drain();
    }
    let result = loop {
        tokio::select! {
            result = &mut run => break result,
            changed = coordinator_status.changed(), if status_open => {
                if changed.is_err() {
                    status_open = false;
                    continue;
                }
                let current = *coordinator_status.borrow();
                if current == GatewayServiceCoordinatorStatus::Ready && !startup_finished {
                    startup_finished = finish_startup(&capacity, token);
                    let _ = status.send(GatewayServiceSupervisorJobStatus::Ready);
                } else if current == GatewayServiceCoordinatorStatus::Starting || current == GatewayServiceCoordinatorStatus::Preparing || current == GatewayServiceCoordinatorStatus::Probing {
                    let _ = status.send(GatewayServiceSupervisorJobStatus::Starting);
                }
            }
            () = cancellation.cancelled(), if !cancel_sent => {
                cancel_sent = true;
                control.cancel();
            }
            changed = drain.changed(), if drain_open => {
                match changed {
                    Ok(()) if *drain.borrow() => control.request_drain(),
                    Ok(()) => {}
                    Err(_) => drain_open = false,
                }
            }
        }
    };
    if !startup_finished {
        startup_finished = finish_startup(&capacity, token);
    }
    match result {
        Ok(()) => terminal(
            id,
            token,
            &capacity,
            &status,
            GatewayServiceSupervisorJobStatus::Settled,
            None,
            startup_finished,
            true,
            None,
        ),
        Err(failure) => {
            let released = failure.physical_cleanup_complete && failure.durable_cleanup_complete;
            let status_value = if cancellation.is_cancelled() {
                GatewayServiceSupervisorJobStatus::Cancelled
            } else {
                GatewayServiceSupervisorJobStatus::Failed
            };
            terminal(
                id,
                token,
                &capacity,
                &status,
                status_value,
                Some(failure.lease.clone()),
                startup_finished,
                released,
                Some(failure),
            )
        }
    }
}

fn finish_startup(
    capacity: &Arc<Mutex<GatewayServiceCapacity>>,
    token: GatewayServiceCapacityToken,
) -> bool {
    capacity
        .lock()
        .expect("service capacity lock")
        .finish_startup(token)
        .is_ok()
}

fn complete_capacity(
    capacity: &Arc<Mutex<GatewayServiceCapacity>>,
    token: GatewayServiceCapacityToken,
) -> bool {
    capacity
        .lock()
        .expect("service capacity lock")
        .complete(token)
        .is_ok()
}

// The terminal record keeps every ownership-bearing value together. Its
// argument count is deliberate: callers must state cleanup completion before
// releasing capacity.
#[allow(clippy::too_many_arguments)]
fn terminal(
    id: Uuid,
    token: GatewayServiceCapacityToken,
    capacity: &Arc<Mutex<GatewayServiceCapacity>>,
    status: &watch::Sender<GatewayServiceSupervisorJobStatus>,
    status_value: GatewayServiceSupervisorJobStatus,
    lease: Option<GatewayServiceInstanceLease>,
    startup_finished: bool,
    release_capacity: bool,
    coordinator_failure: Option<GatewayServiceCoordinatorFailure>,
) -> JobCompletion {
    if !startup_finished {
        let _ = finish_startup(capacity, token);
    }
    let capacity_released = release_capacity && complete_capacity(capacity, token);
    let _ = status.send(status_value);
    JobCompletion {
        id,
        status: status_value,
        capacity_released,
        terminal: JobTerminal {
            lease,
            claim_uncertain: status_value == GatewayServiceSupervisorJobStatus::Uncertain,
            coordinator_failure,
            claim_cleanup_reason: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, VecDeque},
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use ::time::{Duration as TimeDuration, OffsetDateTime};
    use async_trait::async_trait;
    use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::Notify;
    use vm_trait::{
        BoxedPrivateServiceConnection, GuestCommand, NetworkMode, PrivateHttpServiceSpec,
        RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider,
        VmResources,
    };

    use super::*;
    use crate::{
        GatewayEdgeError, GatewayServiceFailure, GatewayServiceFailureStoreError,
        GatewayServiceIdentity, GatewayServiceInstancePage, GatewayServiceInstancePageResult,
        GatewayServiceTargetPage, GatewayServiceTargetPageResult, GatewayServiceTargetStore,
    };

    struct Noop {
        claim_result:
            Mutex<Option<Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>>>,
        claim_started: Notify,
        claim_release: Notify,
        block_claim: AtomicBool,
        claim_calls: AtomicUsize,
    }

    struct FixedClaimResolution {
        result: Mutex<
            Option<Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>>,
        >,
        calls: AtomicUsize,
    }

    struct BlockingClaimResolution {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl GatewayServiceClaimResolutionStore for FixedClaimResolution {
        async fn resolve_revision_claim(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.result
                .lock()
                .expect("claim resolution result")
                .clone()
                .unwrap_or(Ok(None))
        }
    }

    #[async_trait]
    impl GatewayServiceClaimResolutionStore for BlockingClaimResolution {
        async fn resolve_revision_claim(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
            self.started.notify_one();
            self.release.notified().await;
            Err(GatewayServiceOwnershipError::Unavailable)
        }
    }

    impl Default for Noop {
        fn default() -> Self {
            Self {
                claim_result: Mutex::new(None),
                claim_started: Notify::new(),
                claim_release: Notify::new(),
                block_claim: AtomicBool::new(false),
                claim_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl GatewayServiceOwnership for Noop {
        async fn claim_new(
            &self,
            _: Uuid,
            _: Uuid,
            _: &GatewayServiceOwner,
            _: std::time::Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.claim_calls.fetch_add(1, Ordering::Relaxed);
            self.claim_started.notify_one();
            if self.block_claim.load(Ordering::Relaxed) {
                self.claim_release.notified().await;
            }
            self.claim_result
                .lock()
                .expect("claim result")
                .clone()
                .unwrap_or_else(|| unimplemented!())
        }

        async fn renew(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: std::time::Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            unimplemented!()
        }

        async fn claim_expired(
            &self,
            _: &GatewayServiceOwner,
            _: std::time::Duration,
            _: usize,
        ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
            unimplemented!()
        }

        async fn mark_stopping(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            unimplemented!()
        }

        async fn mark_starting(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            unimplemented!()
        }

        async fn mark_ready(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            unimplemented!()
        }

        async fn mark_draining(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            unimplemented!()
        }

        async fn promote_ready(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
            unimplemented!()
        }

        async fn mark_cleaned(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<(), GatewayServiceOwnershipError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl GatewayServiceFailureStore for Noop {
        async fn record_failure(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: GatewayServiceFailure,
        ) -> Result<(), GatewayServiceFailureStoreError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl GatewayServiceLaunchResolver for Noop {
        async fn resolve_service_launch(
            &self,
            _: crate::GatewayServiceLaunchRequest,
        ) -> Result<crate::GatewayServiceLaunch, GatewayEdgeError> {
            unimplemented!()
        }

        async fn cleanup_service_launch(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<(), GatewayEdgeError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl GatewayServiceTargetStore for Noop {
        async fn list_service_targets(
            &self,
            _: GatewayServiceTargetPage,
        ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
            unimplemented!()
        }

        async fn get_service_target(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<Option<crate::GatewayServiceOwnedTarget>, GatewayEdgeError> {
            unimplemented!()
        }

        async fn count_accepted_service_invocations(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<u64, GatewayEdgeError> {
            unimplemented!()
        }

        async fn count_accepted_service_invocations_for_instance(
            &self,
            _: crate::GatewayServiceInstanceKey,
        ) -> Result<u64, GatewayEdgeError> {
            unimplemented!()
        }

        async fn get_service_instance(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
            unimplemented!()
        }

        async fn list_service_instances(
            &self,
            _: GatewayServiceInstancePage,
        ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl VmProvider for Noop {
        fn name(&self) -> &'static str {
            "supervisor-test-noop"
        }

        async fn provision(
            &self,
            _: vm_trait::VmSpec,
        ) -> Result<Arc<dyn vm_trait::VmInstance>, vm_trait::VmError> {
            unimplemented!()
        }

        async fn cleanup_orphan(&self, _: &vm_trait::VmId) -> Result<(), vm_trait::VmError> {
            unimplemented!()
        }
    }

    struct ServiceReadyVm {
        id: VmId,
        events: tokio::sync::broadcast::Sender<VmEvent>,
        fail_destroy: Arc<AtomicBool>,
        destroy_gate: Arc<Mutex<Option<Arc<Notify>>>>,
        destroy_started: Arc<Mutex<Option<Arc<Notify>>>>,
        destroy_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl VmInstance for ServiceReadyVm {
        fn id(&self) -> &VmId {
            &self.id
        }

        async fn start(&self) -> Result<(), VmError> {
            Ok(())
        }

        async fn stop(&self, _: StopMode) -> Result<(), VmError> {
            Ok(())
        }

        async fn wait(&self) -> Result<VmExit, VmError> {
            std::future::pending().await
        }

        async fn open_private_service_connection(
            &self,
        ) -> Result<BoxedPrivateServiceConnection, VmError> {
            let (client, mut peer) = tokio::io::duplex(4096);
            tokio::spawn(async move {
                let mut request = [0_u8; 2048];
                let _ = peer.read(&mut request).await;
                peer.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await
                .expect("readiness response");
            });
            Ok(Box::new(client))
        }

        fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<VmEvent> {
            self.events.subscribe()
        }

        async fn destroy(&self) -> Result<(), VmError> {
            self.destroy_calls.fetch_add(1, Ordering::Relaxed);
            if self.fail_destroy.load(Ordering::Relaxed) {
                Err(VmError::Unavailable {
                    resource: String::from("test VM"),
                    reason: String::from("deliberate cleanup failure"),
                })
            } else {
                if let Some(started) = self
                    .destroy_started
                    .lock()
                    .expect("destroy started")
                    .as_ref()
                {
                    started.notify_one();
                }
                let gate = self.destroy_gate.lock().expect("destroy gate").clone();
                if let Some(gate) = gate {
                    gate.notified().await;
                }
                Ok(())
            }
        }
    }

    struct ReadyProvider {
        fail_destroy: Arc<AtomicBool>,
        destroy_gate: Arc<Mutex<Option<Arc<Notify>>>>,
        destroy_started: Arc<Mutex<Option<Arc<Notify>>>>,
        destroy_calls: Arc<AtomicUsize>,
        orphan_cleanup_calls: AtomicUsize,
        last_vm: Mutex<Option<Arc<dyn VmInstance>>>,
    }

    #[async_trait]
    impl VmProvider for ReadyProvider {
        fn name(&self) -> &'static str {
            "supervisor-ready-test"
        }

        async fn provision(&self, spec: vm_trait::VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
            let (events, _) = tokio::sync::broadcast::channel(8);
            let vm: Arc<dyn VmInstance> = Arc::new(ServiceReadyVm {
                id: spec.id,
                events,
                fail_destroy: Arc::clone(&self.fail_destroy),
                destroy_gate: Arc::clone(&self.destroy_gate),
                destroy_started: Arc::clone(&self.destroy_started),
                destroy_calls: Arc::clone(&self.destroy_calls),
            });
            *self.last_vm.lock().expect("last VM") = Some(Arc::clone(&vm));
            Ok(vm)
        }

        async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
            let _ = id;
            self.orphan_cleanup_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    struct ReadyResolver {
        launch: crate::GatewayServiceLaunch,
    }

    #[async_trait]
    impl GatewayServiceLaunchResolver for ReadyResolver {
        async fn resolve_service_launch(
            &self,
            _: crate::GatewayServiceLaunchRequest,
        ) -> Result<crate::GatewayServiceLaunch, GatewayEdgeError> {
            Ok(self.launch.clone())
        }

        async fn cleanup_service_launch(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<(), GatewayEdgeError> {
            Ok(())
        }
    }

    struct ReadyFailureStore;

    #[async_trait]
    impl GatewayServiceFailureStore for ReadyFailureStore {
        async fn record_failure(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: GatewayServiceFailure,
        ) -> Result<(), GatewayServiceFailureStoreError> {
            Ok(())
        }
    }

    struct ReadyOwnership {
        lease: Mutex<GatewayServiceInstanceLease>,
        renew_stale: AtomicBool,
    }

    impl ReadyOwnership {
        fn current(&self) -> GatewayServiceInstanceLease {
            self.lease.lock().expect("ready lease").clone()
        }

        fn replace(&self, lease: GatewayServiceInstanceLease) {
            *self.lease.lock().expect("ready lease") = lease;
        }

        fn transition(
            &self,
            state: crate::GatewayServiceInstanceState,
        ) -> GatewayServiceInstanceLease {
            let mut lease = self.lease.lock().expect("ready lease");
            lease.state = state;
            lease.clone()
        }
    }

    #[async_trait]
    impl GatewayServiceOwnership for ReadyOwnership {
        async fn claim_new(
            &self,
            _: Uuid,
            _: Uuid,
            _: &GatewayServiceOwner,
            _: std::time::Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Ok(self.current())
        }

        async fn renew(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: std::time::Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            let lease = self.current();
            if self.renew_stale.load(Ordering::Relaxed)
                || lease.lease_expires_at <= OffsetDateTime::now_utc()
            {
                return Err(GatewayServiceOwnershipError::StaleLease);
            }
            Ok(lease)
        }

        async fn claim_expired(
            &self,
            _: &GatewayServiceOwner,
            _: std::time::Duration,
            _: usize,
        ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
            Ok(Vec::new())
        }

        async fn mark_stopping(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Ok(self.transition(crate::GatewayServiceInstanceState::Stopping))
        }

        async fn mark_starting(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Ok(self.transition(crate::GatewayServiceInstanceState::Starting))
        }

        async fn mark_ready(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Ok(self.transition(crate::GatewayServiceInstanceState::Ready))
        }

        async fn mark_draining(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            Ok(self.transition(crate::GatewayServiceInstanceState::Draining))
        }

        async fn promote_ready(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
            Ok(None)
        }

        async fn mark_cleaned(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
        ) -> Result<(), GatewayServiceOwnershipError> {
            let _ = self.transition(crate::GatewayServiceInstanceState::Cleaned);
            Ok(())
        }
    }

    struct FixedExpiredRecovery {
        ownership: Arc<ReadyOwnership>,
        takeover:
            Mutex<VecDeque<Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>>>,
        resolution: Mutex<
            VecDeque<Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>>,
        >,
        takeover_calls: AtomicUsize,
        resolution_calls: AtomicUsize,
    }

    #[async_trait]
    impl GatewayServiceExpiredClaimRecovery for FixedExpiredRecovery {
        async fn resolve_exact_instance(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
            self.resolution_calls.fetch_add(1, Ordering::Relaxed);
            let result = self
                .resolution
                .lock()
                .expect("resolution result")
                .pop_front()
                .unwrap_or(Ok(None));
            if let Ok(Some(lease)) = &result {
                self.ownership.replace(lease.clone());
            }
            result
        }

        async fn claim_expired_instance(
            &self,
            _: &GatewayServiceInstanceLease,
            _: &GatewayServiceOwner,
            _: std::time::Duration,
        ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
            self.takeover_calls.fetch_add(1, Ordering::Relaxed);
            let result = self
                .takeover
                .lock()
                .expect("takeover result")
                .pop_front()
                .unwrap_or(Err(GatewayServiceOwnershipError::Unavailable));
            if let Ok(lease) = &result {
                self.ownership.replace(lease.clone());
            }
            result
        }
    }

    struct ReadyTargets {
        target: crate::GatewayServiceOwnedTarget,
        ownership: Arc<ReadyOwnership>,
        accepted: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl GatewayServiceTargetStore for ReadyTargets {
        async fn list_service_targets(
            &self,
            _: GatewayServiceTargetPage,
        ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
            Ok(GatewayServiceTargetPageResult {
                targets: Vec::new(),
                next_after: None,
            })
        }

        async fn get_service_target(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<Option<crate::GatewayServiceOwnedTarget>, GatewayEdgeError> {
            Ok(Some(self.target.clone()))
        }

        async fn count_accepted_service_invocations(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<u64, GatewayEdgeError> {
            Ok(0)
        }

        async fn count_accepted_service_invocations_for_instance(
            &self,
            _: crate::GatewayServiceInstanceKey,
        ) -> Result<u64, GatewayEdgeError> {
            Ok(self.accepted.load(Ordering::Relaxed) as u64)
        }

        async fn get_service_instance(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
            Ok(Some(self.ownership.current()))
        }

        async fn list_service_instances(
            &self,
            _: GatewayServiceInstancePage,
        ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
            Ok(GatewayServiceInstancePageResult {
                instances: Vec::new(),
                next_after: None,
            })
        }
    }

    fn supervisor_with_policy(
        ownership: Arc<Noop>,
        owner: GatewayServiceOwner,
        policy: GatewayServiceSupervisorPolicy,
    ) -> GatewayServiceSupervisor {
        let registry =
            GatewayServiceRegistry::new(8, policy.requests_per_instance).expect("registry");
        GatewayServiceSupervisor::new(GatewayServiceSupervisorContext {
            owner,
            policy,
            ownership,
            failure_store: Arc::new(Noop::default()),
            resolver: Arc::new(Noop::default()),
            provider: Arc::new(Noop::default()),
            targets: Arc::new(Noop::default()),
            registry,
            service_authority: String::from("127.0.0.1:8080"),
        })
        .expect("supervisor")
    }

    fn supervisor_with(ownership: Arc<Noop>) -> GatewayServiceSupervisor {
        supervisor_with_policy(
            ownership,
            GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner"),
            GatewayServiceSupervisorPolicy::default(),
        )
    }

    fn supervisor() -> GatewayServiceSupervisor {
        supervisor_with(Arc::new(Noop::default()))
    }

    fn lease(
        request: GatewayServiceStartupRequest,
        owner: &GatewayServiceOwner,
    ) -> GatewayServiceInstanceLease {
        let identity = GatewayServiceIdentity {
            instance_id: Uuid::new_v4(),
            gateway_id: request.gateway_id,
            revision_id: request.revision_id,
        };
        GatewayServiceInstanceLease {
            identity,
            owner_host_id: owner.host_id.clone(),
            owner_uuid: owner.owner_uuid,
            fencing_token: 1,
            state: crate::GatewayServiceInstanceState::Provisioning,
            vm_id: format!("gateway-service-{}", identity.instance_id),
            lease_expires_at: OffsetDateTime::now_utc() + TimeDuration::minutes(1),
            heartbeat_at: OffsetDateTime::now_utc(),
        }
    }

    #[allow(clippy::too_many_lines)] // Fixture keeps the retryable VM graph explicit.
    fn ready_supervisor(
        fail_destroy: bool,
    ) -> (
        GatewayServiceSupervisor,
        Arc<ReadyProvider>,
        Arc<ReadyOwnership>,
        GatewayServiceStartupRequest,
        Arc<AtomicUsize>,
    ) {
        let gateway_id = Uuid::new_v4();
        let revision_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let identity = GatewayServiceIdentity {
            instance_id,
            gateway_id,
            revision_id,
        };
        let owner = GatewayServiceOwner::new("ready-host", Uuid::new_v4()).expect("owner");
        let service = GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/ready").expect("readiness"),
            ServiceProbePath::parse("/health").expect("health"),
        )
        .expect("service");
        let launch = crate::GatewayServiceLaunch {
            identity,
            service: service.clone(),
            spec: vm_trait::VmSpec {
                id: VmId(format!("gateway-service-{instance_id}")),
                root: RootFilesystem::Directory {
                    host_path: PathBuf::from("/tmp/supervisor-ready-root"),
                },
                disks: Vec::new(),
                mounts: Vec::new(),
                resources: VmResources {
                    vcpus: 1,
                    memory_mib: 64,
                },
                network: NetworkMode::Disabled,
                command: GuestCommand {
                    program: String::from("/service"),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    working_dir: None,
                },
                runtime_authority: None,
                runtime_git_bridge: None,
                private_http_service: Some(PrivateHttpServiceSpec {
                    loopback_port: 8080,
                    max_connections: 32,
                    connect_timeout: std::time::Duration::from_secs(2),
                }),
                labels: BTreeMap::new(),
            },
        };
        let lease = GatewayServiceInstanceLease {
            identity,
            owner_host_id: owner.host_id.clone(),
            owner_uuid: owner.owner_uuid,
            fencing_token: 1,
            state: crate::GatewayServiceInstanceState::Provisioning,
            vm_id: format!("gateway-service-{instance_id}"),
            lease_expires_at: OffsetDateTime::now_utc() + TimeDuration::minutes(5),
            heartbeat_at: OffsetDateTime::now_utc(),
        };
        let ownership = Arc::new(ReadyOwnership {
            lease: Mutex::new(lease),
            renew_stale: AtomicBool::new(false),
        });
        let fail_destroy = Arc::new(AtomicBool::new(fail_destroy));
        let provider = Arc::new(ReadyProvider {
            fail_destroy: Arc::clone(&fail_destroy),
            destroy_gate: Arc::new(Mutex::new(None)),
            destroy_started: Arc::new(Mutex::new(None)),
            destroy_calls: Arc::new(AtomicUsize::new(0)),
            orphan_cleanup_calls: AtomicUsize::new(0),
            last_vm: Mutex::new(None),
        });
        let target = crate::GatewayServiceOwnedTarget {
            gateway_id,
            lifecycle: String::from("enabled"),
            active_revision_id: None,
            desired_service_revision_id: Some(revision_id),
            revision: crate::GatewayServiceRevisionTarget {
                revision_id,
                release_id: Some(Uuid::new_v4()),
                release_state: Some(String::from("published")),
                publication_eligible: true,
                service,
            },
        };
        let accepted = Arc::new(AtomicUsize::new(0));
        let policy = GatewayServiceSupervisorPolicy::default();
        let registry =
            GatewayServiceRegistry::new(8, policy.requests_per_instance).expect("registry");
        let supervisor = GatewayServiceSupervisor::new(GatewayServiceSupervisorContext {
            owner,
            policy,
            ownership: Arc::clone(&ownership) as Arc<dyn GatewayServiceOwnership>,
            failure_store: Arc::new(ReadyFailureStore),
            resolver: Arc::new(ReadyResolver { launch }),
            provider: Arc::clone(&provider) as Arc<dyn VmProvider>,
            targets: Arc::new(ReadyTargets {
                target,
                ownership: Arc::clone(&ownership),
                accepted: Arc::clone(&accepted),
            }),
            registry,
            service_authority: String::from("127.0.0.1:8080"),
        })
        .expect("supervisor");
        (
            supervisor,
            provider,
            ownership,
            GatewayServiceStartupRequest {
                gateway_id,
                revision_id,
                intent: GatewayServiceStartupIntent::ActivateDesired,
            },
            accepted,
        )
    }

    fn request(gateway_id: Uuid, revision_id: Uuid) -> GatewayServiceStartupRequest {
        GatewayServiceStartupRequest {
            gateway_id,
            revision_id,
            intent: GatewayServiceStartupIntent::ActivateDesired,
        }
    }

    fn insert_known_claim_record(
        supervisor: &mut GatewayServiceSupervisor,
        request: GatewayServiceStartupRequest,
        lease: &GatewayServiceInstanceLease,
    ) -> Uuid {
        let token = supervisor
            .capacity
            .lock()
            .expect("capacity")
            .reserve(request.gateway_id, request.revision_id)
            .expect("capacity reservation");
        let id = Uuid::new_v4();
        let (status, _) = watch::channel(GatewayServiceSupervisorJobStatus::Uncertain);
        supervisor.records.insert(
            id,
            JobRecord {
                request,
                token,
                cancellation: CancellationToken::new(),
                status,
                capacity_retained: true,
                completion: Some(JobTerminal {
                    lease: Some(lease.clone()),
                    claim_uncertain: false,
                    coordinator_failure: None,
                    claim_cleanup_reason: Some(GatewayServiceCoordinatorFailureReason::Cancelled),
                }),
                cleanup_retry: None,
                cleanup_in_flight: false,
                claim_resolution_in_flight: false,
            },
        );
        id
    }

    async fn wait_until_ready(
        supervisor: &mut GatewayServiceSupervisor,
        handle: &GatewayServiceStartupHandle,
    ) {
        let mut observed = handle.subscribe();
        let poll = supervisor.poll();
        tokio::pin!(poll);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    changed = observed.changed() => {
                        changed.expect("status channel");
                        if *observed.borrow() == GatewayServiceSupervisorJobStatus::Ready {
                            break;
                        }
                    }
                    event = &mut poll => panic!("startup ended before readiness: {event:?}"),
                }
            }
        })
        .await
        .expect("bounded readiness");
    }

    #[test]
    fn start_reserves_capacity_before_claim_future_is_polled() {
        let mut supervisor = supervisor();
        let request = request(Uuid::new_v4(), Uuid::new_v4());
        let handle = supervisor.start(request).expect("reservation");
        assert_eq!(
            handle.subscribe().borrow().to_owned(),
            GatewayServiceSupervisorJobStatus::Claiming
        );
        assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
        assert_eq!(supervisor.capacity_snapshot().starting_instances, 1);
        drop(supervisor);
    }

    #[test]
    fn startup_capacity_is_bounded_before_any_claim_is_polled() {
        let mut supervisor = supervisor();
        for _ in 0..2 {
            supervisor
                .start(request(Uuid::new_v4(), Uuid::new_v4()))
                .expect("startup slot");
        }
        let error = supervisor
            .start(request(Uuid::new_v4(), Uuid::new_v4()))
            .expect_err("third startup must wait");
        assert_eq!(
            error,
            GatewayServiceSupervisorError::Capacity(
                GatewayServiceCapacityError::StartupCapacityExhausted
            )
        );
        drop(supervisor);
    }

    #[tokio::test]
    async fn cancellation_before_first_poll_releases_without_claiming() {
        let ownership = Arc::new(Noop::default());
        let mut supervisor = supervisor_with(Arc::clone(&ownership));
        let handle = supervisor
            .start(request(Uuid::new_v4(), Uuid::new_v4()))
            .expect("reservation");
        handle.cancel();
        let event = supervisor.poll().await.expect("cancelled job");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Cancelled);
        assert!(event.capacity_released);
        assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 0);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
        drop(supervisor);
    }

    #[tokio::test]
    async fn expired_queued_deadline_releases_without_claiming() {
        let ownership = Arc::new(Noop::default());
        let policy = GatewayServiceSupervisorPolicy {
            instance: crate::ServiceInstancePolicy::new(
                std::time::Duration::from_millis(1),
                std::time::Duration::from_millis(1),
                std::time::Duration::from_millis(1),
                std::time::Duration::from_secs(1),
            ),
            ..GatewayServiceSupervisorPolicy::default()
        };
        let mut supervisor = supervisor_with_policy(
            Arc::clone(&ownership),
            GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner"),
            policy,
        );
        supervisor
            .start(request(Uuid::new_v4(), Uuid::new_v4()))
            .expect("reservation");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let event = supervisor.poll().await.expect("expired job");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Failed);
        assert!(event.capacity_released);
        assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 0);
        drop(supervisor);
    }

    // `shutdown` consumes the supervisor after joining every owned future; the
    // nursery lint cannot see that this is the deliberate final drop point.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test]
    async fn successful_wrong_revision_claim_is_retained_without_provisioning() {
        let ownership = Arc::new(Noop::default());
        let owner = GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner");
        let requested = request(Uuid::new_v4(), Uuid::new_v4());
        let wrong = request(requested.gateway_id, Uuid::new_v4());
        let claimed = lease(wrong, &owner);
        *ownership.claim_result.lock().expect("claim result") = Some(Ok(claimed.clone()));
        let mut supervisor = supervisor_with_policy(
            Arc::clone(&ownership),
            owner,
            GatewayServiceSupervisorPolicy::default(),
        );
        supervisor.start(requested).expect("reservation");
        let event = supervisor.poll().await.expect("invalid claim job");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Failed);
        assert!(!event.capacity_released);
        let shutdown = supervisor.shutdown().await;
        assert_eq!(shutdown.unresolved.len(), 1);
        assert_eq!(shutdown.unresolved[0].lease.as_ref(), Some(&claimed));
        assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn coordinator_readiness_releases_startup_but_cleanup_failure_retains_vm() {
        let (mut supervisor, provider, _ownership, initial_request, _accepted) =
            ready_supervisor(true);
        let handle = supervisor.start(initial_request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        let snapshot = supervisor.capacity_snapshot();
        assert_eq!(snapshot.live_instances, 1);
        assert_eq!(snapshot.starting_instances, 0);
        handle.cancel();
        let event = supervisor.poll().await.expect("cleanup result");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Cancelled);
        assert!(!event.capacity_released);
        let expected_vm = provider.last_vm.lock().expect("last VM").clone();
        let shutdown = supervisor.shutdown().await;
        assert_eq!(shutdown.unresolved.len(), 1);
        let failure = shutdown.unresolved[0]
            .coordinator_failure
            .as_ref()
            .expect("retained coordinator failure");
        let actual_vm = failure.vm.as_ref().expect("retained VM");
        assert!(Arc::ptr_eq(
            actual_vm,
            expected_vm.as_ref().expect("expected VM")
        ));
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
        assert!(!failure.physical_cleanup_complete);
    }

    #[tokio::test]
    async fn terminal_cleanup_can_retry_with_the_same_capacity_and_vm() {
        let (mut supervisor, provider, _ownership, initial_request, _accepted) =
            ready_supervisor(true);
        let handle = supervisor.start(initial_request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        handle.cancel();
        let first = supervisor.poll().await.expect("initial cleanup result");
        assert_eq!(first.status, GatewayServiceSupervisorJobStatus::Cancelled);
        assert!(!first.capacity_released);
        let retained = provider.last_vm.lock().expect("last VM").clone();
        let recorded = supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.coordinator_failure.as_ref())
            .and_then(|failure| failure.vm.as_ref())
            .expect("recorded retained VM");
        assert!(Arc::ptr_eq(
            retained.as_ref().expect("retained VM"),
            recorded
        ));
        provider.fail_destroy.store(false, Ordering::Relaxed);
        supervisor
            .retry_cleanup(first.job_id)
            .expect("cleanup retry scheduling");
        assert_eq!(
            handle.subscribe().borrow().to_owned(),
            GatewayServiceSupervisorJobStatus::CleanupPending
        );
        assert_eq!(
            supervisor
                .retry_cleanup(first.job_id)
                .expect_err("duplicate retry"),
            GatewayServiceSupervisorError::RetryAlreadyInFlight
        );
        let retried = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
            .await
            .expect("bounded cleanup retry")
            .expect("retry result");
        assert_eq!(retried.status, GatewayServiceSupervisorJobStatus::Settled);
        assert!(retried.capacity_released);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
        assert!(Arc::ptr_eq(
            retained.as_ref().expect("retained VM"),
            provider
                .last_vm
                .lock()
                .expect("last VM")
                .as_ref()
                .expect("VM")
        ));
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
        assert!(!supervisor.has_pending_jobs());
    }

    #[tokio::test]
    async fn expired_cleanup_retains_successor_until_renewal_then_retries() {
        let (mut supervisor, provider, ownership, initial_request, _accepted) =
            ready_supervisor(true);
        let handle = supervisor.start(initial_request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        handle.cancel();
        let first = supervisor.poll().await.expect("initial cleanup result");
        assert!(!first.capacity_released);
        provider.fail_destroy.store(false, Ordering::Relaxed);

        let old = supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.clone())
            .expect("retained lease");
        let owner = supervisor.context.owner.clone();
        let now = OffsetDateTime::now_utc();
        let mut expired_successor = old.clone();
        expired_successor.owner_host_id = owner.host_id.clone();
        expired_successor.owner_uuid = owner.owner_uuid;
        expired_successor.fencing_token = old.fencing_token + 1;
        expired_successor.state = GatewayServiceInstanceState::Stopping;
        expired_successor.heartbeat_at = now - TimeDuration::seconds(2);
        expired_successor.lease_expires_at = now - TimeDuration::seconds(1);
        let mut live_successor = expired_successor.clone();
        live_successor.fencing_token = expired_successor.fencing_token + 1;
        live_successor.heartbeat_at = now;
        live_successor.lease_expires_at = now + TimeDuration::minutes(1);
        let recovery = Arc::new(FixedExpiredRecovery {
            ownership: Arc::clone(&ownership),
            takeover: Mutex::new(VecDeque::from([
                Ok(expired_successor.clone()),
                Ok(live_successor.clone()),
            ])),
            resolution: Mutex::new(VecDeque::new()),
            takeover_calls: AtomicUsize::new(0),
            resolution_calls: AtomicUsize::new(0),
        });
        ownership.renew_stale.store(true, Ordering::Relaxed);

        supervisor
            .retry_cleanup_with_recovery(first.job_id, recovery.clone())
            .expect("expired cleanup retry");
        let retained = supervisor.poll().await.expect("expired retry result");
        assert_eq!(retained.status, GatewayServiceSupervisorJobStatus::Failed);
        assert!(!retained.capacity_released);
        assert_eq!(
            supervisor
                .records
                .get(&first.job_id)
                .and_then(|record| record.completion.as_ref())
                .and_then(|completion| completion.lease.as_ref()),
            Some(&expired_successor)
        );
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);

        ownership.renew_stale.store(false, Ordering::Relaxed);
        supervisor
            .retry_cleanup_with_recovery(first.job_id, recovery.clone())
            .expect("eventual cleanup retry");
        let cleaned = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
            .await
            .expect("bounded eventual cleanup")
            .expect("cleanup result");
        assert_eq!(cleaned.status, GatewayServiceSupervisorJobStatus::Settled);
        assert!(cleaned.capacity_released);
        assert_eq!(recovery.takeover_calls.load(Ordering::Relaxed), 2);
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    }

    #[tokio::test]
    async fn expired_cleanup_retries_lost_takeover_and_resolution_acknowledgements() {
        let (mut supervisor, provider, ownership, initial_request, _accepted) =
            ready_supervisor(true);
        let handle = supervisor.start(initial_request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        handle.cancel();
        let first = supervisor.poll().await.expect("initial cleanup result");
        assert!(!first.capacity_released);
        provider.fail_destroy.store(false, Ordering::Relaxed);

        let old = supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.clone())
            .expect("retained lease");
        let owner = supervisor.context.owner.clone();
        let mut successor = old.clone();
        successor.owner_host_id = owner.host_id.clone();
        successor.owner_uuid = owner.owner_uuid;
        successor.fencing_token = old.fencing_token + 1;
        successor.state = GatewayServiceInstanceState::Stopping;
        successor.heartbeat_at = OffsetDateTime::now_utc();
        successor.lease_expires_at = successor.heartbeat_at + TimeDuration::minutes(1);
        let recovery = Arc::new(FixedExpiredRecovery {
            ownership: Arc::clone(&ownership),
            takeover: Mutex::new(VecDeque::from([
                Err(GatewayServiceOwnershipError::Unavailable),
                Err(GatewayServiceOwnershipError::StaleLease),
            ])),
            resolution: Mutex::new(VecDeque::from([
                Err(GatewayServiceOwnershipError::Unavailable),
                Ok(Some(successor.clone())),
            ])),
            takeover_calls: AtomicUsize::new(0),
            resolution_calls: AtomicUsize::new(0),
        });
        ownership.renew_stale.store(true, Ordering::Relaxed);

        supervisor
            .retry_cleanup_with_recovery(first.job_id, recovery.clone())
            .expect("first recovery retry");
        let first_retry = supervisor.poll().await.expect("first retry result");
        assert_eq!(
            first_retry.status,
            GatewayServiceSupervisorJobStatus::Failed
        );
        assert!(!first_retry.capacity_released);
        assert_eq!(recovery.takeover_calls.load(Ordering::Relaxed), 1);
        assert_eq!(recovery.resolution_calls.load(Ordering::Relaxed), 1);

        supervisor
            .retry_cleanup_with_recovery(first.job_id, recovery.clone())
            .expect("second recovery retry");
        let second_retry = supervisor.poll().await.expect("second retry result");
        assert_eq!(
            second_retry.status,
            GatewayServiceSupervisorJobStatus::Failed
        );
        assert!(!second_retry.capacity_released);
        assert_eq!(recovery.takeover_calls.load(Ordering::Relaxed), 2);
        assert_eq!(recovery.resolution_calls.load(Ordering::Relaxed), 2);
        assert_eq!(
            supervisor
                .records
                .get(&first.job_id)
                .and_then(|record| record.completion.as_ref())
                .and_then(|completion| completion.lease.as_ref()),
            Some(&successor)
        );

        ownership.renew_stale.store(false, Ordering::Relaxed);
        supervisor
            .retry_cleanup_with_recovery(first.job_id, recovery)
            .expect("final cleanup retry");
        let cleaned = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
            .await
            .expect("bounded final cleanup")
            .expect("cleanup result");
        assert_eq!(cleaned.status, GatewayServiceSupervisorJobStatus::Settled);
        assert!(cleaned.capacity_released);
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn expired_cleanup_rejects_unrelated_successor_without_physical_work() {
        let (mut supervisor, provider, ownership, initial_request, _accepted) =
            ready_supervisor(true);
        let handle = supervisor.start(initial_request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        handle.cancel();
        let first = supervisor.poll().await.expect("initial cleanup result");
        assert!(!first.capacity_released);
        provider.fail_destroy.store(false, Ordering::Relaxed);
        ownership.renew_stale.store(true, Ordering::Relaxed);
        let retained_vm = provider.last_vm.lock().expect("last VM").clone();
        let destroy_calls = provider.destroy_calls.load(Ordering::Relaxed);

        let old = supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.clone())
            .expect("retained lease");
        let mut unrelated = old.clone();
        unrelated.vm_id = String::from("gateway-service-foreign");
        let recovery = Arc::new(FixedExpiredRecovery {
            ownership: Arc::clone(&ownership),
            takeover: Mutex::new(VecDeque::from([Ok(unrelated)])),
            resolution: Mutex::new(VecDeque::new()),
            takeover_calls: AtomicUsize::new(0),
            resolution_calls: AtomicUsize::new(0),
        });
        supervisor
            .retry_cleanup_with_recovery(first.job_id, recovery)
            .expect("recovery retry");
        let rejected = supervisor.poll().await.expect("recovery result");
        assert_eq!(rejected.status, GatewayServiceSupervisorJobStatus::Failed);
        assert!(!rejected.capacity_released);
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
        let recorded_vm = supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.coordinator_failure.as_ref())
            .and_then(|failure| failure.vm.as_ref())
            .expect("recorded retained VM");
        assert!(Arc::ptr_eq(
            retained_vm.as_ref().expect("retained VM"),
            recorded_vm
        ));
        assert_eq!(
            provider.destroy_calls.load(Ordering::Relaxed),
            destroy_calls
        );
        assert_eq!(
            supervisor
                .records
                .get(&first.job_id)
                .and_then(|record| record.completion.as_ref())
                .and_then(|completion| completion.lease.as_ref()),
            Some(&old)
        );
    }

    #[tokio::test]
    async fn expired_cleanup_rejects_cleaned_successor_before_physical_confirmation() {
        let (mut supervisor, provider, ownership, initial_request, _accepted) =
            ready_supervisor(true);
        let handle = supervisor.start(initial_request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        handle.cancel();
        let first = supervisor.poll().await.expect("initial cleanup result");
        assert!(!first.capacity_released);
        provider.fail_destroy.store(false, Ordering::Relaxed);
        ownership.renew_stale.store(true, Ordering::Relaxed);
        let retained_vm = provider.last_vm.lock().expect("last VM").clone();
        let destroy_calls = provider.destroy_calls.load(Ordering::Relaxed);

        let old = supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.lease.clone())
            .expect("retained lease");
        let recovery = Arc::new(FixedExpiredRecovery {
            ownership: Arc::clone(&ownership),
            takeover: Mutex::new(VecDeque::from([Ok(GatewayServiceInstanceLease {
                state: GatewayServiceInstanceState::Cleaned,
                fencing_token: old.fencing_token + 1,
                ..old.clone()
            })])),
            resolution: Mutex::new(VecDeque::new()),
            takeover_calls: AtomicUsize::new(0),
            resolution_calls: AtomicUsize::new(0),
        });
        supervisor
            .retry_cleanup_with_recovery(first.job_id, recovery)
            .expect("recovery retry");
        let rejected = supervisor.poll().await.expect("recovery result");
        assert_eq!(rejected.status, GatewayServiceSupervisorJobStatus::Failed);
        assert!(!rejected.capacity_released);
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
        let recorded_vm = supervisor
            .records
            .get(&first.job_id)
            .and_then(|record| record.completion.as_ref())
            .and_then(|completion| completion.coordinator_failure.as_ref())
            .and_then(|failure| failure.vm.as_ref())
            .expect("recorded retained VM");
        assert!(Arc::ptr_eq(
            retained_vm.as_ref().expect("retained VM"),
            recorded_vm
        ));
        assert_eq!(
            provider.destroy_calls.load(Ordering::Relaxed),
            destroy_calls
        );
        assert_eq!(
            supervisor
                .records
                .get(&first.job_id)
                .and_then(|record| record.completion.as_ref())
                .and_then(|completion| completion.lease.as_ref()),
            Some(&old)
        );
    }

    #[tokio::test]
    async fn expired_cleanup_keeps_state_for_absent_foreign_newer_or_rolled_back_resolution() {
        #[derive(Clone, Copy)]
        enum ResolutionCase {
            Absent,
            Foreign,
            Newer,
            RolledBack,
        }

        for case in [
            ResolutionCase::Absent,
            ResolutionCase::Foreign,
            ResolutionCase::Newer,
            ResolutionCase::RolledBack,
        ] {
            let (mut supervisor, provider, ownership, initial_request, _accepted) =
                ready_supervisor(true);
            let handle = supervisor.start(initial_request).expect("reservation");
            wait_until_ready(&mut supervisor, &handle).await;
            handle.cancel();
            let first = supervisor.poll().await.expect("initial cleanup result");
            assert!(!first.capacity_released);
            provider.fail_destroy.store(false, Ordering::Relaxed);
            ownership.renew_stale.store(true, Ordering::Relaxed);

            let old = supervisor
                .records
                .get(&first.job_id)
                .and_then(|record| record.completion.as_ref())
                .and_then(|completion| completion.lease.clone())
                .expect("retained lease");
            let resolved = match case {
                ResolutionCase::Absent => Ok(None),
                ResolutionCase::Foreign => {
                    let mut candidate = old.clone();
                    candidate.owner_host_id = String::from("other-host");
                    candidate.owner_uuid = Uuid::new_v4();
                    Ok(Some(candidate))
                }
                ResolutionCase::Newer => {
                    let mut candidate = old.clone();
                    candidate.fencing_token += 2;
                    Ok(Some(candidate))
                }
                ResolutionCase::RolledBack => Ok(Some(old.clone())),
            };
            let recovery = Arc::new(FixedExpiredRecovery {
                ownership: Arc::clone(&ownership),
                takeover: Mutex::new(VecDeque::from([Err(
                    GatewayServiceOwnershipError::Unavailable,
                )])),
                resolution: Mutex::new(VecDeque::from([resolved])),
                takeover_calls: AtomicUsize::new(0),
                resolution_calls: AtomicUsize::new(0),
            });
            supervisor
                .retry_cleanup_with_recovery(first.job_id, recovery)
                .expect("recovery retry");
            let rejected = supervisor.poll().await.expect("recovery result");
            assert_eq!(rejected.status, GatewayServiceSupervisorJobStatus::Failed);
            assert!(!rejected.capacity_released);
            assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
            assert_eq!(
                supervisor
                    .records
                    .get(&first.job_id)
                    .and_then(|record| record.completion.as_ref())
                    .and_then(|completion| completion.lease.as_ref()),
                Some(&old)
            );
        }
    }

    #[tokio::test]
    async fn blocked_cleanup_retry_does_not_stop_other_startup_jobs() {
        let (mut supervisor, provider, _ownership, initial_request, _accepted) =
            ready_supervisor(true);
        let handle = supervisor.start(initial_request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        handle.cancel();
        let first = supervisor.poll().await.expect("initial cleanup result");
        assert!(!first.capacity_released);
        provider.fail_destroy.store(false, Ordering::Relaxed);
        let gate = Arc::new(Notify::new());
        let started = Arc::new(Notify::new());
        *provider.destroy_gate.lock().expect("destroy gate") = Some(Arc::clone(&gate));
        *provider.destroy_started.lock().expect("destroy started") = Some(Arc::clone(&started));
        supervisor
            .retry_cleanup(first.job_id)
            .expect("retry scheduling");
        {
            let poll = supervisor.poll();
            tokio::pin!(poll);
            tokio::select! {
                () = started.notified() => {}
                event = &mut poll => panic!("cleanup ended before blocking: {event:?}"),
            }
        }

        let other_request = request(Uuid::new_v4(), Uuid::new_v4());
        let other = supervisor.start(other_request).expect("other startup slot");
        let other_event =
            tokio::time::timeout(std::time::Duration::from_secs(1), supervisor.poll())
                .await
                .expect("other job remains schedulable")
                .expect("other job result");
        assert_eq!(other_event.job_id, other.job_id());
        assert_eq!(
            other_event.status,
            GatewayServiceSupervisorJobStatus::Failed
        );
        assert!(supervisor.has_pending_jobs());
        gate.notify_one();
        let cleanup_event =
            tokio::time::timeout(std::time::Duration::from_secs(1), supervisor.poll())
                .await
                .expect("cleanup retry settles")
                .expect("cleanup result");
        assert_eq!(cleanup_event.job_id, first.job_id);
        assert!(cleanup_event.capacity_released);
    }

    #[tokio::test]
    async fn coordinator_success_releases_live_capacity_after_cleanup() {
        let (mut supervisor, _provider, _ownership, request, _accepted) = ready_supervisor(false);
        let handle = supervisor.start(request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;
        assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
        assert_eq!(supervisor.capacity_snapshot().starting_instances, 0);
        handle.cancel();
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
            .await
            .expect("bounded cleanup")
            .expect("cleanup result");
        assert!(event.capacity_released);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
        assert!(!supervisor.has_pending_jobs());
        let shutdown = supervisor.shutdown().await;
        assert!(shutdown.unresolved.is_empty());
    }

    #[tokio::test]
    async fn startup_handle_forwards_drain_to_ready_coordinator() {
        let (mut supervisor, _provider, _ownership, request, accepted) = ready_supervisor(false);
        let handle = supervisor.start(request).expect("reservation");
        wait_until_ready(&mut supervisor, &handle).await;

        accepted.store(1, Ordering::Relaxed);
        handle.request_drain();
        let mut poll = Box::pin(supervisor.poll());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut poll)
                .await
                .is_err()
        );
        assert_eq!(
            *handle.subscribe().borrow(),
            GatewayServiceSupervisorJobStatus::Ready
        );
        accepted.store(0, Ordering::Relaxed);
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), &mut poll)
            .await
            .expect("bounded graceful drain")
            .expect("drained service event");
        drop(poll);
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Settled);
        assert!(event.capacity_released);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    }

    #[tokio::test]
    async fn startup_handle_retains_drain_requested_before_coordinator_creation() {
        let (mut supervisor, _provider, _ownership, request, _accepted) = ready_supervisor(false);
        let handle = supervisor.start(request).expect("reservation");
        handle.request_drain();

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), supervisor.poll())
            .await
            .expect("bounded pre-start drain")
            .expect("drained service event");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Settled);
        assert!(event.capacity_released);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
    }

    // `shutdown` consumes the supervisor after joining every owned future; the
    // nursery lint cannot see that this is the deliberate final drop point.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test]
    async fn cancellation_after_claim_started_retains_late_lease_and_capacity() {
        let ownership = Arc::new(Noop::default());
        ownership.block_claim.store(true, Ordering::Relaxed);
        let request = request(Uuid::new_v4(), Uuid::new_v4());
        let owner = GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner");
        let claimed = lease(request, &owner);
        *ownership.claim_result.lock().expect("claim result") = Some(Ok(claimed.clone()));
        let mut supervisor = supervisor_with_policy(
            Arc::clone(&ownership),
            owner,
            GatewayServiceSupervisorPolicy::default(),
        );
        let handle = supervisor.start(request).expect("reservation");
        let event = {
            let poll = supervisor.poll();
            tokio::pin!(poll);
            tokio::select! {
                () = ownership.claim_started.notified() => {}
                _ = &mut poll => panic!("claim should be blocked before cancellation"),
            }
            handle.cancel();
            ownership.claim_release.notify_one();
            poll.await.expect("cancelled job")
        };
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Cancelled);
        assert!(!event.capacity_released);
        let shutdown = supervisor.shutdown().await;
        assert_eq!(shutdown.unresolved.len(), 1);
        assert_eq!(shutdown.unresolved[0].request, request);
        assert_eq!(shutdown.unresolved[0].lease.as_ref(), Some(&claimed));
        assert_eq!(
            shutdown.unresolved[0].original_reason,
            Some(GatewayServiceCoordinatorFailureReason::Cancelled)
        );
    }

    // `shutdown` consumes the supervisor after joining every owned future; the
    // nursery lint cannot see that this is the deliberate final drop point.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test]
    async fn unavailable_claim_is_quarantined_until_reconciliation() {
        let ownership = Arc::new(Noop::default());
        *ownership.claim_result.lock().expect("claim result") =
            Some(Err(GatewayServiceOwnershipError::Unavailable));
        let mut supervisor = supervisor_with(Arc::clone(&ownership));
        let request = request(Uuid::new_v4(), Uuid::new_v4());
        supervisor.start(request).expect("reservation");
        let event = supervisor.poll().await.expect("uncertain job");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Uncertain);
        assert!(!event.capacity_released);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
        assert_eq!(supervisor.capacity_snapshot().starting_instances, 0);
        assert_eq!(
            supervisor.start(request).unwrap_err(),
            GatewayServiceSupervisorError::Duplicate
        );
        let shutdown = supervisor.shutdown().await;
        assert_eq!(shutdown.unresolved.len(), 1);
        assert_eq!(shutdown.unresolved[0].request, request);
    }

    #[tokio::test]
    async fn serialized_absence_releases_an_uncertain_claim_reservation() {
        let ownership = Arc::new(Noop::default());
        *ownership.claim_result.lock().expect("claim result") =
            Some(Err(GatewayServiceOwnershipError::Unavailable));
        let mut supervisor = supervisor_with(ownership);
        let request = request(Uuid::new_v4(), Uuid::new_v4());
        supervisor.start(request).expect("reservation");
        let uncertain = supervisor.poll().await.expect("uncertain claim");
        assert_eq!(
            uncertain.status,
            GatewayServiceSupervisorJobStatus::Uncertain
        );
        let resolver = Arc::new(FixedClaimResolution {
            result: Mutex::new(Some(Ok(None))),
            calls: AtomicUsize::new(0),
        });
        supervisor
            .reconcile_claim(uncertain.job_id, resolver.clone())
            .expect("resolution scheduling");
        let resolution_event = supervisor.poll().await.expect("resolution result");
        assert_eq!(
            resolution_event.status,
            GatewayServiceSupervisorJobStatus::Settled
        );
        assert!(resolution_event.capacity_released);
        assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
        drop(supervisor);
    }

    #[tokio::test]
    async fn foreign_resolved_claim_remains_retained_without_cleanup() {
        let ownership = Arc::new(Noop::default());
        *ownership.claim_result.lock().expect("claim result") =
            Some(Err(GatewayServiceOwnershipError::Unavailable));
        let mut supervisor = supervisor_with(Arc::clone(&ownership));
        let request = request(Uuid::new_v4(), Uuid::new_v4());
        supervisor.start(request).expect("reservation");
        let uncertain = supervisor.poll().await.expect("uncertain claim");
        let owner = GatewayServiceOwner::new("foreign-host", Uuid::new_v4()).expect("owner");
        let foreign = lease(request, &owner);
        let resolver = Arc::new(FixedClaimResolution {
            result: Mutex::new(Some(Ok(Some(foreign)))),
            calls: AtomicUsize::new(0),
        });
        supervisor
            .reconcile_claim(uncertain.job_id, resolver.clone())
            .expect("resolution scheduling");
        let resolution_event = supervisor.poll().await.expect("resolution result");
        assert_eq!(
            resolution_event.status,
            GatewayServiceSupervisorJobStatus::Uncertain
        );
        assert!(!resolution_event.capacity_released);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
        assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 1);
        supervisor
            .reconcile_claim(resolution_event.job_id, resolver)
            .expect("repeat resolution scheduling");
        let repeated_event = supervisor.poll().await.expect("repeat resolution result");
        assert_eq!(
            repeated_event.status,
            GatewayServiceSupervisorJobStatus::Uncertain
        );
        assert_eq!(ownership.claim_calls.load(Ordering::Relaxed), 1);
        drop(supervisor);
    }

    #[tokio::test]
    async fn known_claim_resolution_retries_preserve_original_lease() {
        #[derive(Clone, Copy)]
        enum ResolutionCase {
            NewerFence,
            WrongInstance,
            WrongVmId,
            Expired,
            Unavailable,
        }

        for case in [
            ResolutionCase::NewerFence,
            ResolutionCase::WrongInstance,
            ResolutionCase::WrongVmId,
            ResolutionCase::Expired,
            ResolutionCase::Unavailable,
        ] {
            let (mut supervisor, provider, _ownership, request, _accepted) =
                ready_supervisor(false);
            let known = supervisor
                .context
                .targets
                .get_service_instance(GatewayServiceIdentity {
                    instance_id: Uuid::nil(),
                    gateway_id: request.gateway_id,
                    revision_id: request.revision_id,
                })
                .await
                .expect("known target lookup")
                .expect("known lease");
            let result = match case {
                ResolutionCase::NewerFence => {
                    let mut candidate = known.clone();
                    candidate.fencing_token += 1;
                    Ok(Some(candidate))
                }
                ResolutionCase::WrongInstance => {
                    let mut candidate = known.clone();
                    candidate.identity.instance_id = Uuid::new_v4();
                    candidate.vm_id = format!("gateway-service-{}", candidate.identity.instance_id);
                    Ok(Some(candidate))
                }
                ResolutionCase::WrongVmId => {
                    let mut candidate = known.clone();
                    candidate.vm_id = String::from("gateway-service-wrong");
                    Ok(Some(candidate))
                }
                ResolutionCase::Expired => {
                    let mut candidate = known.clone();
                    candidate.lease_expires_at =
                        OffsetDateTime::now_utc() - TimeDuration::seconds(1);
                    Ok(Some(candidate))
                }
                ResolutionCase::Unavailable => Err(GatewayServiceOwnershipError::Unavailable),
            };
            let resolver = Arc::new(FixedClaimResolution {
                result: Mutex::new(Some(result)),
                calls: AtomicUsize::new(0),
            });
            let id = insert_known_claim_record(&mut supervisor, request, &known);
            for _ in 0..2 {
                supervisor
                    .reconcile_claim(id, resolver.clone())
                    .expect("resolution scheduling");
                let event = supervisor.poll().await.expect("resolution result");
                assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Uncertain);
                assert!(!event.capacity_released);
            }
            let record = supervisor.records.get(&id).expect("retained record");
            assert_eq!(
                record
                    .completion
                    .as_ref()
                    .and_then(|terminal| terminal.lease.as_ref()),
                Some(&known)
            );
            assert_eq!(supervisor.capacity_snapshot().live_instances, 1);
            assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 0);
            drop(supervisor);
        }
    }

    // This test intentionally drops the supervisor after proving a ready
    // resolution is not starved by an unrelated pending cleanup future.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test]
    async fn poll_services_resolution_while_cleanup_is_pending() {
        let ownership = Arc::new(Noop::default());
        *ownership.claim_result.lock().expect("claim result") =
            Some(Err(GatewayServiceOwnershipError::Unavailable));
        let mut supervisor = supervisor_with(ownership);
        let request = request(Uuid::new_v4(), Uuid::new_v4());
        supervisor.start(request).expect("reservation");
        let uncertain = supervisor.poll().await.expect("uncertain claim");
        supervisor
            .cleanup_jobs
            .push(Box::pin(std::future::pending::<CleanupCompletion>()));
        let resolver = Arc::new(FixedClaimResolution {
            result: Mutex::new(Some(Ok(None))),
            calls: AtomicUsize::new(0),
        });
        supervisor
            .reconcile_claim(uncertain.job_id, resolver)
            .expect("resolution scheduling");
        let event = tokio::time::timeout(std::time::Duration::from_millis(100), supervisor.poll())
            .await
            .expect("resolution should not be starved")
            .expect("resolution event");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Settled);
        assert!(event.capacity_released);
        drop(supervisor);
    }

    // The supervisor owns the blocked resolution future; this test drops only
    // the poll wait, then consumes the supervisor after releasing that future.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test]
    async fn dropping_resolution_poll_wait_keeps_future_for_shutdown() {
        let ownership = Arc::new(Noop::default());
        *ownership.claim_result.lock().expect("claim result") =
            Some(Err(GatewayServiceOwnershipError::Unavailable));
        let mut supervisor = supervisor_with(ownership);
        let request = request(Uuid::new_v4(), Uuid::new_v4());
        supervisor.start(request).expect("reservation");
        let uncertain = supervisor.poll().await.expect("uncertain claim");
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        supervisor
            .reconcile_claim(
                uncertain.job_id,
                Arc::new(BlockingClaimResolution {
                    started: Arc::clone(&started),
                    release: Arc::clone(&release),
                }),
            )
            .expect("resolution scheduling");
        {
            let poll = supervisor.poll();
            tokio::pin!(poll);
            tokio::select! {
                () = started.notified() => {}
                _ = &mut poll => panic!("resolution should remain blocked"),
            }
        }
        release.notify_one();
        let event = supervisor.poll().await.expect("resolution result");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Uncertain);
        let shutdown = supervisor.shutdown().await;
        assert_eq!(shutdown.unresolved.len(), 1);
        assert_eq!(shutdown.unresolved[0].request, request);
    }

    #[tokio::test]
    async fn exact_resolved_owned_claim_is_cleaned_before_capacity_release() {
        let (mut supervisor, provider, _ownership, request, _accepted) = ready_supervisor(false);
        let lease = supervisor
            .context
            .targets
            .get_service_instance(GatewayServiceIdentity {
                instance_id: Uuid::new_v4(),
                gateway_id: request.gateway_id,
                revision_id: request.revision_id,
            })
            .await
            .expect("target lookup")
            .expect("ready fixture lease");
        let token = supervisor
            .capacity
            .lock()
            .expect("capacity")
            .reserve(request.gateway_id, request.revision_id)
            .expect("capacity reservation");
        let id = Uuid::new_v4();
        let (status, _) = watch::channel(GatewayServiceSupervisorJobStatus::Uncertain);
        supervisor.records.insert(
            id,
            JobRecord {
                request,
                token,
                cancellation: CancellationToken::new(),
                status,
                capacity_retained: true,
                completion: Some(JobTerminal {
                    lease: Some(lease.clone()),
                    claim_uncertain: false,
                    coordinator_failure: None,
                    claim_cleanup_reason: Some(GatewayServiceCoordinatorFailureReason::Cancelled),
                }),
                cleanup_retry: None,
                cleanup_in_flight: false,
                claim_resolution_in_flight: false,
            },
        );
        let resolver = Arc::new(FixedClaimResolution {
            result: Mutex::new(Some(Ok(Some(lease)))),
            calls: AtomicUsize::new(0),
        });
        supervisor
            .reconcile_claim(id, resolver.clone())
            .expect("resolution scheduling");
        let pending = supervisor.poll().await.expect("cleanup scheduled");
        assert_eq!(
            pending.status,
            GatewayServiceSupervisorJobStatus::CleanupPending
        );
        assert_eq!(
            supervisor.reconcile_claim(id, resolver),
            Err(GatewayServiceSupervisorError::RetryAlreadyInFlight)
        );
        let cleaned = supervisor.poll().await.expect("cleanup result");
        assert_eq!(cleaned.status, GatewayServiceSupervisorJobStatus::Settled);
        assert!(cleaned.capacity_released);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
        assert_eq!(provider.orphan_cleanup_calls.load(Ordering::Relaxed), 1);
        drop(supervisor);
    }

    #[tokio::test]
    async fn definite_claim_conflict_releases_capacity() {
        let ownership = Arc::new(Noop::default());
        *ownership.claim_result.lock().expect("claim result") =
            Some(Err(GatewayServiceOwnershipError::Conflict));
        let mut supervisor = supervisor_with(Arc::clone(&ownership));
        let request = request(Uuid::new_v4(), Uuid::new_v4());
        supervisor.start(request).expect("reservation");
        let event = supervisor.poll().await.expect("conflict job");
        assert_eq!(event.status, GatewayServiceSupervisorJobStatus::Failed);
        assert!(event.capacity_released);
        assert_eq!(supervisor.capacity_snapshot().live_instances, 0);
        drop(supervisor);
    }

    #[tokio::test]
    async fn dropping_poll_wait_keeps_parent_owned_future() {
        let notify = Arc::new(Notify::new());
        let waiter = Arc::clone(&notify);
        let mut jobs: Vec<Pin<Box<dyn Future<Output = JobCompletion> + Send>>> =
            vec![Box::pin(async move {
                waiter.notified().await;
                JobCompletion {
                    id: Uuid::new_v4(),
                    status: GatewayServiceSupervisorJobStatus::Settled,
                    capacity_released: true,
                    terminal: JobTerminal {
                        lease: None,
                        claim_uncertain: false,
                        coordinator_failure: None,
                        claim_cleanup_reason: None,
                    },
                }
            })];
        {
            let pending = poll_next_job(&mut jobs);
            tokio::pin!(pending);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(5), &mut pending)
                    .await
                    .is_err()
            );
        }
        assert_eq!(jobs.len(), 1);
        notify.notify_one();
        assert!(poll_next_job(&mut jobs).await.is_some());
        assert!(jobs.is_empty());
    }
}

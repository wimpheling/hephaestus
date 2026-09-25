use super::gateway_service_state::{
    SERVICE_CLEANUP_RETRY_INITIAL_BACKOFF, TrackedServiceJob, tracked_job,
};
use super::{
    Arc, GatewayServiceClaimResolutionStore, GatewayServiceExpiredClaimRecovery, HashMap, Instant,
    Uuid,
};
use gateway_edge::{
    GatewayServiceStartupRequest, GatewayServiceSupervisor, GatewayServiceSupervisorJobStatus,
};

pub fn schedule_one_cleanup_retry(
    supervisor: &mut GatewayServiceSupervisor,
    tracked_jobs: &mut HashMap<Uuid, TrackedServiceJob>,
    cursor: &mut Option<Uuid>,
    claim_resolution: Option<&Arc<dyn GatewayServiceClaimResolutionStore>>,
    expired_claim_recovery: Option<&Arc<dyn GatewayServiceExpiredClaimRecovery>>,
) -> bool {
    let pending = tracked_jobs
        .values()
        .filter(|job| {
            *job.handle.subscribe().borrow() == GatewayServiceSupervisorJobStatus::CleanupPending
        })
        .count();
    if pending >= 2 {
        return false;
    }

    let now = Instant::now();
    let mut ids = tracked_jobs.keys().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    let Some(job_id) = cursor
        .and_then(|last| ids.iter().copied().find(|id| *id > last))
        .or_else(|| ids.first().copied())
    else {
        return false;
    };
    let ordered = ids
        .iter()
        .copied()
        .cycle()
        .skip_while(|id| *id != job_id)
        .take(ids.len())
        .collect::<Vec<_>>();
    for candidate in ordered {
        let Some(job) = tracked_jobs.get_mut(&candidate) else {
            continue;
        };
        let due = job
            .cleanup_retry_due
            .is_some_and(|deadline| deadline <= now);
        if !due {
            continue;
        }
        *cursor = Some(candidate);
        let retry_result = if let Some(recovery) = expired_claim_recovery {
            supervisor.retry_cleanup_with_recovery(candidate, Arc::clone(recovery))
        } else {
            supervisor.retry_cleanup(candidate)
        };
        match retry_result {
            Ok(()) => {
                job.cleanup_retry_due = None;
                return true;
            }
            Err(gateway_edge::GatewayServiceSupervisorError::RetryNotEligible) => {
                // A claim acknowledgement may have been lost before the
                // coordinator returned. Resolve it behind the serialized
                // gateway barrier instead of silently stranding its capacity.
                let resolved = claim_resolution.is_some_and(|resolver| {
                    match supervisor.reconcile_claim(candidate, Arc::clone(resolver)) {
                        Ok(()) => true,
                        Err(gateway_edge::GatewayServiceSupervisorError::RetryAlreadyInFlight) => {
                            job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
                            false
                        }
                        Err(
                            gateway_edge::GatewayServiceSupervisorError::RetryNotFound
                            | gateway_edge::GatewayServiceSupervisorError::RetryNotEligible,
                        ) => {
                            job.cleanup_retry_due = None;
                            false
                        }
                        Err(error) => {
                            tracing::debug!(
                                job_id = %candidate,
                                %error,
                                "gateway service claim reconciliation was not scheduled"
                            );
                            job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
                            false
                        }
                    }
                });
                if resolved {
                    job.cleanup_retry_due = None;
                    return true;
                }
                if claim_resolution.is_none() {
                    job.cleanup_retry_due = None;
                }
            }
            Err(gateway_edge::GatewayServiceSupervisorError::RetryNotFound) => {
                job.cleanup_retry_due = None;
            }
            Err(gateway_edge::GatewayServiceSupervisorError::RetryAlreadyInFlight) => {
                job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
            }
            Err(error) => {
                tracing::debug!(job_id = %candidate, %error, "gateway service cleanup retry was not scheduled");
                job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
            }
        }
    }
    false
}

pub fn start_service_job(
    supervisor: &mut GatewayServiceSupervisor,
    tracked_jobs: &mut HashMap<Uuid, TrackedServiceJob>,
    request: GatewayServiceStartupRequest,
) {
    if tracked_job(tracked_jobs, request.gateway_id, request.revision_id).is_some() {
        return;
    }
    match supervisor.start(request) {
        Ok(handle) => {
            let job_id = handle.job_id();
            tracked_jobs.insert(
                job_id,
                TrackedServiceJob {
                    gateway_id: request.gateway_id,
                    revision_id: request.revision_id,
                    handle,
                    retirement_requested: false,
                    cleanup_retry_due: None,
                    cleanup_retry_backoff: SERVICE_CLEANUP_RETRY_INITIAL_BACKOFF,
                    cleanup_retry_attempted: false,
                },
            );
        }
        Err(error) => {
            tracing::debug!(
                gateway_id = %request.gateway_id,
                revision_id = %request.revision_id,
                %error,
                "gateway service startup was not admitted"
            );
        }
    }
}

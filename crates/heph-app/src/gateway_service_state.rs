use super::gateway_service_cleanup::start_service_job;
use super::{Arc, Duration, HashMap, Instant, PostgresGatewayEdgeAuthority, Uuid};
use gateway_edge::{
    GatewayProvider, GatewayServiceStartupIntent, GatewayServiceStartupRequest,
    GatewayServiceSupervisor, GatewayServiceSupervisorContext, GatewayServiceSupervisorJobStatus,
    GatewayServiceTargetPageResult,
};

pub const GATEWAY_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(1);
pub const GATEWAY_CADDY_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

pub const SERVICE_TARGET_REFRESH_TIMEOUT: Duration = Duration::from_secs(2);
pub const SERVICE_CLEANUP_RETRY_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
pub const SERVICE_CLEANUP_RETRY_MAX_BACKOFF: Duration = Duration::from_secs(30);

pub struct TrackedServiceJob {
    pub gateway_id: Uuid,
    pub revision_id: Uuid,
    pub handle: gateway_edge::GatewayServiceStartupHandle,
    pub retirement_requested: bool,
    pub cleanup_retry_due: Option<Instant>,
    pub cleanup_retry_backoff: Duration,
    pub cleanup_retry_attempted: bool,
}

pub fn clone_service_supervisor_context(
    context: &Arc<GatewayServiceSupervisorContext>,
) -> GatewayServiceSupervisorContext {
    GatewayServiceSupervisorContext {
        owner: context.owner.clone(),
        policy: context.policy,
        ownership: Arc::clone(&context.ownership),
        failure_store: Arc::clone(&context.failure_store),
        resolver: Arc::clone(&context.resolver),
        provider: Arc::clone(&context.provider),
        targets: Arc::clone(&context.targets),
        registry: context.registry.clone(),
        service_authority: context.service_authority.clone(),
    }
}

pub async fn reconcile_gateway_once(
    authority: PostgresGatewayEdgeAuthority,
    provider: Arc<dyn GatewayProvider>,
    recover: bool,
) {
    let desired = match authority.desired_configuration().await {
        Ok(desired) => desired,
        Err(error) => {
            tracing::warn!(%error, "gateway desired-route reconstruction failed");
            return;
        }
    };
    let result = if recover {
        provider.recover(&desired).await
    } else {
        provider.reconcile(&desired).await
    };
    if let Err(error) = result {
        tracing::warn!(%error, recovery = recover, "gateway Caddy reconciliation failed");
    }
}

/// Reconciles one bounded page of durable service targets.
///
/// The active service is restored first. A desired replacement is admitted
/// only after that active service reports `Ready`; the supervisor owns durable
/// promotion and the coordinator owns exact drain counts.
pub fn reconcile_service_target_page(
    supervisor: &mut GatewayServiceSupervisor,
    tracked_jobs: &mut HashMap<Uuid, TrackedServiceJob>,
    page: GatewayServiceTargetPageResult,
    allow_startups: bool,
) {
    for target in page.targets {
        if target.lifecycle != "enabled" {
            continue;
        }
        let active_is_eligible = target
            .active_service_revision
            .as_ref()
            .is_some_and(|active| {
                target.active_revision_id == Some(active.revision_id) && active.publication_eligible
            });
        if allow_startups
            && let Some(active) = target.active_service_revision.as_ref()
            && active_is_eligible
        {
            start_service_job(
                supervisor,
                tracked_jobs,
                GatewayServiceStartupRequest {
                    gateway_id: target.gateway_id,
                    revision_id: active.revision_id,
                    intent: GatewayServiceStartupIntent::RestoreActive,
                },
            );
        }
        let Some(desired) = target.desired_service_revision.as_ref() else {
            continue;
        };
        if !desired.publication_eligible || target.active_revision_id == Some(desired.revision_id) {
            continue;
        }
        let active_ready = active_is_eligible
            && target
                .active_service_revision
                .as_ref()
                .and_then(|active| tracked_job(tracked_jobs, target.gateway_id, active.revision_id))
                .is_some_and(|job| {
                    *job.handle.subscribe().borrow() == GatewayServiceSupervisorJobStatus::Ready
                });
        let revoked_active_settled = !active_is_eligible
            && target
                .active_service_revision
                .as_ref()
                .is_none_or(|active| {
                    tracked_job(tracked_jobs, target.gateway_id, active.revision_id).is_none()
                });
        let gateway_job_count = tracked_jobs
            .values()
            .filter(|job| job.gateway_id == target.gateway_id)
            .count();
        if allow_startups
            && (active_ready || target.active_service_revision.is_none() || revoked_active_settled)
            && gateway_job_count < 2
        {
            start_service_job(
                supervisor,
                tracked_jobs,
                GatewayServiceStartupRequest {
                    gateway_id: target.gateway_id,
                    revision_id: desired.revision_id,
                    intent: GatewayServiceStartupIntent::ActivateDesired,
                },
            );
        }
    }
}

pub fn tracked_job(
    tracked_jobs: &HashMap<Uuid, TrackedServiceJob>,
    gateway_id: Uuid,
    revision_id: Uuid,
) -> Option<&TrackedServiceJob> {
    tracked_jobs
        .values()
        .find(|job| job.gateway_id == gateway_id && job.revision_id == revision_id)
}

pub fn next_tracked_job_id(
    tracked_jobs: &HashMap<Uuid, TrackedServiceJob>,
    cursor: Option<Uuid>,
) -> Option<Uuid> {
    let mut ids = tracked_jobs.keys().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    cursor
        .and_then(|cursor| ids.iter().copied().find(|id| *id > cursor))
        .or_else(|| ids.first().copied())
}

pub fn reconcile_tracked_service_job(
    job: &mut TrackedServiceJob,
    target: &gateway_edge::GatewayServiceOwnedTarget,
) {
    let candidate_is_still_desired = target.desired_service_revision_id == Some(job.revision_id);
    let target_is_current = target.active_revision_id == Some(job.revision_id);
    if !target.revision.publication_eligible {
        job.handle.cancel();
        job.retirement_requested = true;
        return;
    }
    if job.retirement_requested {
        let status = *job.handle.subscribe().borrow();
        if target.lifecycle == "enabled"
            && target_is_current
            && status == GatewayServiceSupervisorJobStatus::Ready
        {
            // A stale paused/superseded read may have sent a drain request
            // just before the gateway became current again. Clear that
            // request marker while the worker is still Ready so a later
            // exact retirement read can retry after a durable Conflict.
            job.retirement_requested = false;
        } else if status == GatewayServiceSupervisorJobStatus::Ready {
            // `mark_draining` can reject a stale target read after a
            // concurrent gateway transition. The next paced exact refresh
            // must be able to submit the request again.
            job.handle.request_drain();
        }
        return;
    }
    if target.lifecycle != "enabled" {
        retire_tracked_service_job(job);
        return;
    }
    if !target_is_current && candidate_is_still_desired {
        // A desired candidate remains available until its own coordinator
        // promotes it; it is not obsolete merely because A still serves.
        return;
    }
    if !target_is_current {
        retire_tracked_service_job(job);
    }
}

pub fn retire_tracked_service_job(job: &mut TrackedServiceJob) {
    if job.retirement_requested {
        return;
    }
    if *job.handle.subscribe().borrow() == GatewayServiceSupervisorJobStatus::Ready {
        job.handle.request_drain();
    } else {
        job.handle.cancel();
    }
    job.retirement_requested = true;
}

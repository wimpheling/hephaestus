use super::gateway_service_cleanup::schedule_one_cleanup_retry;
use super::gateway_service_state::{
    GATEWAY_CADDY_RECOVERY_INTERVAL, GATEWAY_RECONCILIATION_INTERVAL,
    SERVICE_CLEANUP_RETRY_MAX_BACKOFF, SERVICE_TARGET_REFRESH_TIMEOUT, TrackedServiceJob,
    next_tracked_job_id, reconcile_gateway_once, reconcile_service_target_page,
    reconcile_tracked_service_job,
};
use super::{
    Arc, CancellationToken, Future, HashMap, Instant, JoinHandle, OffsetDateTime,
    PostgresGatewayEdgeAuthority, Uuid,
};
use gateway_edge::{
    GatewayProvider, GatewayServiceBootRecovery, GatewayServiceClaimResolutionStore,
    GatewayServiceExpiredClaimRecovery, GatewayServiceSupervisor, GatewayServiceTargetPage,
    GatewayServiceTargetPageResult,
};
use std::pin::Pin;

type GatewayServiceTargetScan = Pin<
    Box<
        dyn Future<Output = Result<GatewayServiceTargetPageResult, gateway_edge::GatewayEdgeError>>
            + Send,
    >,
>;

type GatewayServiceTargetRefresh = Pin<
    Box<
        dyn Future<
                Output = (
                    Uuid,
                    Uuid,
                    Uuid,
                    Result<
                        Option<gateway_edge::GatewayServiceOwnedTarget>,
                        gateway_edge::GatewayEdgeError,
                    >,
                ),
            > + Send,
    >,
>;

// The single select set is deliberate: it keeps Caddy, recovery, service
// jobs, and shutdown under one parent-owned polling boundary.
// The loop receives separately owned adapters so each parent-polled subsystem
// keeps its cancellation and lifetime boundary explicit.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn gateway_reconciliation_loop_with_boot(
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    mut service_supervisor: GatewayServiceSupervisor,
    mut boot_recovery: Option<GatewayServiceBootRecovery>,
    service_claim_resolution: Option<Arc<dyn GatewayServiceClaimResolutionStore>>,
    service_expired_claim_recovery: Option<Arc<dyn GatewayServiceExpiredClaimRecovery>>,
    service_targets: Arc<dyn gateway_edge::GatewayServiceTargetStore>,
    provider: Arc<dyn GatewayProvider>,
    cancellation: CancellationToken,
) {
    let mut reconcile = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut service_recovery = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    service_recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Avoid an immediate duplicate of the startup reconciliation while still
    // making a daemon-owned Caddy restart recover without operator action.
    let mut recovery = tokio::time::interval_at(
        tokio::time::Instant::now() + GATEWAY_CADDY_RECOVERY_INTERVAL,
        GATEWAY_CADDY_RECOVERY_INTERVAL,
    );
    recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut service_recovery_task: Option<JoinHandle<()>> = None;
    let mut caddy_task = None;
    let mut caddy_recovery_pending = false;
    let mut target_scan: Option<GatewayServiceTargetScan> = None;
    let mut target_scan_after = None;
    let mut target_scan_interval = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    target_scan_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut tracked_jobs = HashMap::<Uuid, TrackedServiceJob>::new();
    let mut target_refresh: Option<GatewayServiceTargetRefresh> = None;
    let mut target_refresh_cursor = None;
    let mut target_refresh_interval = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    target_refresh_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut cleanup_retry_interval = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    cleanup_retry_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut cleanup_retry_cursor = None;
    loop {
        tokio::select! {
            () = cancellation.cancelled() => {
                if let Some(task) = service_recovery_task.take() {
                    task.abort();
                    let _ = task.await;
                }
                if let Some(boot_recovery) = boot_recovery.take() {
                    let boot_shutdown = boot_recovery.shutdown().await;
                    if !boot_shutdown.unresolved.is_empty()
                        || !boot_shutdown.unresolved_claims.is_empty()
                    {
                        tracing::warn!(
                            unresolved = boot_shutdown.unresolved.len()
                                + boot_shutdown.unresolved_claims.len(),
                            "gateway service boot recovery retained unresolved shutdown work"
                        );
                    }
                }
                let shutdown = service_supervisor.shutdown().await;
                if !shutdown.unresolved.is_empty() {
                    tracing::warn!(
                        unresolved = shutdown.unresolved.len(),
                        "gateway service supervisor retained unresolved shutdown work"
                    );
                }
                return;
            },
            _ = reconcile.tick() => {
                if caddy_task.is_none() {
                    caddy_task = Some(Box::pin(reconcile_gateway_once(
                        authority.clone(),
                        Arc::clone(&provider),
                        false,
                    )));
                }
            }
            _ = recovery.tick() => {
                if caddy_task.is_some() {
                    caddy_recovery_pending = true;
                } else {
                    caddy_task = Some(Box::pin(reconcile_gateway_once(
                        authority.clone(),
                        Arc::clone(&provider),
                        true,
                    )));
                }
            }
            _ = async {
                match caddy_task.as_mut() {
                    Some(task) => {
                        task.await;
                        Some(())
                    },
                    None => std::future::pending().await,
                }
            }, if caddy_task.is_some() => {
                caddy_task = None;
                if caddy_recovery_pending {
                    caddy_recovery_pending = false;
                    caddy_task = Some(Box::pin(reconcile_gateway_once(
                        authority.clone(),
                        Arc::clone(&provider),
                        true,
                    )));
                }
            }
            _ = service_recovery.tick(), if service_recovery_task.is_none() => {
                let authority = recovery_authority.clone();
                service_recovery_task = Some(tokio::spawn(async move {
                    match authority
                        .recover_abandoned_service_invocations(OffsetDateTime::now_utc())
                        .await
                    {
                        Ok(recovered) if recovered > 0 => {
                            tracing::info!(recovered, "recovered abandoned gateway service invocations");
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(%error, "gateway service invocation recovery failed");
                        }
                    }
                }));
            }
            result = async {
                match service_recovery_task.as_mut() {
                    Some(task) => Some(task.await),
                    None => std::future::pending().await,
                }
            }, if service_recovery_task.is_some() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "gateway service invocation recovery task failed");
                }
                service_recovery_task = None;
            }
            boot_event = async {
                match boot_recovery.as_mut() {
                    Some(recovery) => Some(recovery.poll().await),
                    None => std::future::pending().await,
                }
            }, if boot_recovery.as_ref().is_some_and(|recovery| !recovery.is_complete()) => {
                match boot_event {
                    Some(Ok(gateway_edge::GatewayServiceBootRecoveryEvent::Complete)) => {
                        tracing::info!("gateway service boot recovery gate completed");
                    }
                    Some(Ok(
                        gateway_edge::GatewayServiceBootRecoveryEvent::Pending
                        | gateway_edge::GatewayServiceBootRecoveryEvent::Waiting,
                    )) => {}
                    Some(Err(error)) => {
                        tracing::warn!(%error, "gateway service boot recovery is unavailable");
                    }
                    None => unreachable!("boot poll branch is enabled only with a boot gate"),
                }
            }
            _ = target_scan_interval.tick(),
                if boot_recovery.as_ref().is_some_and(GatewayServiceBootRecovery::is_complete)
                    && target_scan.is_none() =>
            {
                let page = match GatewayServiceTargetPage::new(
                    target_scan_after,
                    gateway_edge::MAX_SERVICE_TARGET_PAGE_SIZE,
                ) {
                    Ok(page) => page,
                    Err(error) => {
                        tracing::warn!(%error, "gateway service target page is invalid");
                        continue;
                    }
                };
                let targets = Arc::clone(&service_targets);
                target_scan = Some(Box::pin(async move {
                    targets.list_service_targets(page).await
                }));
            }
            result = async {
                match target_scan.as_mut() {
                    Some(scan) => Some(scan.await),
                    None => std::future::pending().await,
                }
            }, if target_scan.is_some() => {
                target_scan = None;
                match result {
                    Some(Ok(page)) => {
                        target_scan_after = page.next_after;
                        let cleanup_queued = schedule_one_cleanup_retry(
                            &mut service_supervisor,
                            &mut tracked_jobs,
                            &mut cleanup_retry_cursor,
                            service_claim_resolution.as_ref(),
                            service_expired_claim_recovery.as_ref(),
                        );
                        reconcile_service_target_page(
                            &mut service_supervisor,
                            &mut tracked_jobs,
                            page,
                            !cleanup_queued,
                        );
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, "gateway service target scan failed");
                        target_scan_after = None;
                    }
                    None => unreachable!("target scan branch is enabled only with a scan"),
                }
            }
            _ = target_refresh_interval.tick(),
                if target_refresh.is_none() && !tracked_jobs.is_empty() =>
            {
                if let Some(job_id) = next_tracked_job_id(&tracked_jobs, target_refresh_cursor)
                {
                    target_refresh_cursor = Some(job_id);
                    let job = tracked_jobs
                        .get(&job_id)
                        .expect("refresh cursor points at tracked job");
                    let gateway_id = job.gateway_id;
                    let revision_id = job.revision_id;
                    let targets = Arc::clone(&service_targets);
                    target_refresh = Some(Box::pin(async move {
                        let result = tokio::time::timeout(
                            SERVICE_TARGET_REFRESH_TIMEOUT,
                            targets.get_service_target(gateway_id, revision_id),
                        )
                        .await
                        .unwrap_or(Err(gateway_edge::GatewayEdgeError::Unavailable));
                        (job_id, gateway_id, revision_id, result)
                    }));
                }
            }
            result = async {
                match target_refresh.as_mut() {
                    Some(refresh) => Some(refresh.await),
                    None => std::future::pending().await,
                }
            }, if target_refresh.is_some() => {
                target_refresh = None;
                if let Some((job_id, gateway_id, revision_id, result)) = result
                    && let Some(job) = tracked_jobs.get_mut(&job_id)
                    && job.gateway_id == gateway_id
                    && job.revision_id == revision_id
                {
                    match result {
                        Ok(Some(target)) => {
                            reconcile_tracked_service_job(job, &target);
                        }
                        Ok(None) => {
                            job.handle.cancel();
                            job.retirement_requested = true;
                        }
                        Err(error) => {
                            tracing::debug!(
                                job_id = %job_id,
                                gateway_id = %gateway_id,
                                revision_id = %revision_id,
                                %error,
                                "gateway service target refresh is unavailable"
                            );
                        }
                    }
                }
            }
            event = async {
                if service_supervisor.has_pending_jobs() {
                    service_supervisor.poll().await
                } else {
                    std::future::pending().await
                }
            }, if service_supervisor.has_pending_jobs() => {
                if let Some(event) = event {
                    tracing::debug!(
                        job_id = %event.job_id,
                        status = ?event.status,
                        capacity_released = event.capacity_released,
                        "gateway service supervisor job completed"
                    );
                    if event.capacity_released {
                        tracked_jobs.remove(&event.job_id);
                    } else if let Some(job) = tracked_jobs.get_mut(&event.job_id) {
                        let now = Instant::now();
                        if job.cleanup_retry_attempted {
                            job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
                            job.cleanup_retry_backoff = (job.cleanup_retry_backoff * 2)
                                .min(SERVICE_CLEANUP_RETRY_MAX_BACKOFF);
                        } else {
                            job.cleanup_retry_attempted = true;
                            job.cleanup_retry_due = Some(now);
                        }
                    }
                }
            }
            _ = cleanup_retry_interval.tick() => {
                let _ = schedule_one_cleanup_retry(
                    &mut service_supervisor,
                    &mut tracked_jobs,
                    &mut cleanup_retry_cursor,
                    service_claim_resolution.as_ref(),
                    service_expired_claim_recovery.as_ref(),
                );
            }
        }
    }
}

//! Supervised scheduler for bounded service-log retention maintenance.
//!
//! The scheduler owns the enumeration cursor and one bounded maintenance
//! operation at a time.  The application owns its lifetime by awaiting
//! [`GatewayServiceLogMaintenanceScheduler::run`] and cancelling the supplied
//! token during shutdown.

use std::sync::Arc;
use std::time::Duration;

use gateway_edge::{
    GatewayServiceLogMaintenance, GatewayServiceLogMaintenancePolicy,
    GatewayServiceLogMaintenanceProjectPage, GatewayServiceLogMaintenanceProjectPageResult,
    GatewayServiceLogMaintenanceProjects, MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE,
};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const OPERATION_TIMEOUT: Duration = Duration::from_secs(2);
const SWEEP_GAP: Duration = Duration::from_secs(5);
const INITIAL_ENUMERATION_BACKOFF: Duration = Duration::from_secs(1);
const MAX_ENUMERATION_BACKOFF: Duration = Duration::from_secs(60);

/// A malformed page returned by the enumeration port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceLogMaintenanceSchedulerError {
    /// The enumeration port violated its bounded keyset-page contract.
    #[error("invalid gateway service-log maintenance project page: {0}")]
    InvalidPage(&'static str),
}

/// A supervised, bounded service-log maintenance scheduler.
#[derive(Clone)]
pub struct GatewayServiceLogMaintenanceScheduler {
    projects: Arc<dyn GatewayServiceLogMaintenanceProjects>,
    maintenance: Arc<dyn GatewayServiceLogMaintenance>,
    policy: GatewayServiceLogMaintenancePolicy,
}

impl GatewayServiceLogMaintenanceScheduler {
    /// Creates a scheduler over the shared project-enumeration and
    /// maintenance ports.
    #[must_use]
    pub fn new(
        projects: Arc<dyn GatewayServiceLogMaintenanceProjects>,
        maintenance: Arc<dyn GatewayServiceLogMaintenance>,
        policy: GatewayServiceLogMaintenancePolicy,
    ) -> Self {
        Self {
            projects,
            maintenance,
            policy,
        }
    }

    /// Runs supervised sweeps until `cancel` is cancelled.
    ///
    /// Enumeration failures and operation timeouts are retried with a capped
    /// exponential delay.  A project maintenance failure is isolated to that
    /// project and never prevents the cursor from advancing through the
    /// current page.  The in-flight operation is owned by this call and is
    /// dropped on cancellation; no detached task or unbounded queue is used.
    ///
    /// # Errors
    ///
    /// Retains the result shape used by the supervised task boundary; malformed
    /// pages are logged and retried at the current cursor rather than returned.
    pub async fn run(
        &self,
        cancel: CancellationToken,
    ) -> Result<(), GatewayServiceLogMaintenanceSchedulerError> {
        loop {
            let Some(_report) = self.run_sweep(&cancel).await? else {
                return Ok(());
            };
            if !wait_or_cancel(&cancel, SWEEP_GAP).await {
                return Ok(());
            }
        }
    }

    // Keep the bounded retry state machine together so cursor and backoff
    // transitions remain auditable in one place.
    #[allow(clippy::cognitive_complexity)]
    async fn run_sweep(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Option<SweepReport>, GatewayServiceLogMaintenanceSchedulerError> {
        let mut after = None;
        let mut enumeration_backoff = INITIAL_ENUMERATION_BACKOFF;
        let mut report = SweepReport::default();

        loop {
            let page_request = GatewayServiceLogMaintenanceProjectPage::new(
                after,
                MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE,
            )
            .expect("scheduler constructs a bounded page");
            let listed = tokio::select! {
                () = cancel.cancelled() => return Ok(None),
                result = timeout(OPERATION_TIMEOUT, self.projects.list_projects(page_request)) => result,
            };

            let page = match listed {
                Ok(Ok(page)) => {
                    if validate_page(after, &page).is_err() {
                        tracing::warn!(
                            "service-log maintenance enumeration returned an invalid page"
                        );
                        if !wait_or_cancel(cancel, enumeration_backoff).await {
                            return Ok(None);
                        }
                        enumeration_backoff = next_enumeration_backoff(enumeration_backoff);
                        continue;
                    }
                    enumeration_backoff = INITIAL_ENUMERATION_BACKOFF;
                    page
                }
                Ok(Err(_error)) => {
                    tracing::warn!("service-log maintenance enumeration failed; retrying");
                    if !wait_or_cancel(cancel, enumeration_backoff).await {
                        return Ok(None);
                    }
                    enumeration_backoff = next_enumeration_backoff(enumeration_backoff);
                    continue;
                }
                Err(_timeout) => {
                    tracing::warn!("service-log maintenance enumeration timed out; retrying");
                    if !wait_or_cancel(cancel, enumeration_backoff).await {
                        return Ok(None);
                    }
                    enumeration_backoff = next_enumeration_backoff(enumeration_backoff);
                    continue;
                }
            };

            report.pages = report.pages.saturating_add(1);
            report.projects = report.projects.saturating_add(page.projects.len());
            let maintenance = self
                .maintain_page(&page.projects, cancel, &mut report.maintenance_failures)
                .await;
            let Some(has_more) = maintenance else {
                return Ok(None);
            };
            report.has_more |= has_more;

            after = page.next_after;
            if after.is_none() {
                return Ok(Some(report));
            }
        }
    }

    async fn maintain_page(
        &self,
        projects: &[Uuid],
        cancel: &CancellationToken,
        failures: &mut usize,
    ) -> Option<bool> {
        let mut has_more = false;

        for project_id in projects {
            let result = tokio::select! {
                () = cancel.cancelled() => return None,
                result = timeout(
                    OPERATION_TIMEOUT,
                    self.maintenance.maintain_project(*project_id, self.policy),
                ) => result,
            };
            match result {
                Ok(Ok(report)) => has_more |= report.has_more,
                Ok(Err(_error)) => {
                    tracing::warn!("service-log maintenance project failed; continuing sweep");
                    *failures = failures.saturating_add(1);
                }
                Err(_timeout) => {
                    tracing::warn!("service-log maintenance project timed out; continuing sweep");
                    *failures = failures.saturating_add(1);
                }
            }
        }
        Some(has_more)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SweepReport {
    pages: usize,
    projects: usize,
    maintenance_failures: usize,
    has_more: bool,
}

fn validate_page(
    after: Option<Uuid>,
    page: &GatewayServiceLogMaintenanceProjectPageResult,
) -> Result<(), GatewayServiceLogMaintenanceSchedulerError> {
    if page.projects.len() > usize::from(MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE) {
        return Err(GatewayServiceLogMaintenanceSchedulerError::InvalidPage(
            "project page exceeds the bounded limit",
        ));
    }

    let mut previous = after;
    for project_id in &page.projects {
        if project_id.is_nil() {
            return Err(GatewayServiceLogMaintenanceSchedulerError::InvalidPage(
                "project page contains a nil project id",
            ));
        }
        if previous.is_some_and(|cursor| *project_id <= cursor) {
            return Err(GatewayServiceLogMaintenanceSchedulerError::InvalidPage(
                "project page is not strictly after its cursor",
            ));
        }
        previous = Some(*project_id);
    }

    match (page.projects.last(), page.next_after) {
        (None | Some(_), None) => Ok(()),
        (None, Some(_)) => Err(GatewayServiceLogMaintenanceSchedulerError::InvalidPage(
            "empty page has a continuation cursor",
        )),
        (Some(last), Some(next_after)) if next_after == *last => Ok(()),
        (Some(_), Some(_)) => Err(GatewayServiceLogMaintenanceSchedulerError::InvalidPage(
            "continuation cursor is out of order",
        )),
    }
}

fn next_enumeration_backoff(current: Duration) -> Duration {
    current
        .checked_mul(2)
        .unwrap_or(MAX_ENUMERATION_BACKOFF)
        .min(MAX_ENUMERATION_BACKOFF)
}

async fn wait_or_cancel(cancel: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        () = cancel.cancelled() => false,
        () = sleep(delay) => true,
    }
}

#[cfg(test)]
#[path = "service_log_maintenance/tests.rs"]
mod tests;

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
mod tests {
    use super::*;
    use async_trait::async_trait;
    use gateway_edge::{GatewayServiceLogMaintenanceError, GatewayServiceLogMaintenanceReport};
    use std::collections::VecDeque;
    use std::future::pending;
    use std::sync::Mutex;

    struct FakeProjects {
        afters: Arc<Mutex<Vec<Option<Uuid>>>>,
        pages: Mutex<
            VecDeque<
                Result<
                    GatewayServiceLogMaintenanceProjectPageResult,
                    GatewayServiceLogMaintenanceError,
                >,
            >,
        >,
        hang: bool,
    }

    #[async_trait]
    impl GatewayServiceLogMaintenanceProjects for FakeProjects {
        async fn list_projects(
            &self,
            page: GatewayServiceLogMaintenanceProjectPage,
        ) -> Result<GatewayServiceLogMaintenanceProjectPageResult, GatewayServiceLogMaintenanceError>
        {
            self.afters
                .lock()
                .expect("fake cursor lock")
                .push(page.after);
            if self.hang {
                pending().await
            }
            self.pages
                .lock()
                .expect("fake project lock")
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(GatewayServiceLogMaintenanceProjectPageResult {
                        projects: Vec::new(),
                        next_after: None,
                    })
                })
        }
    }

    struct FakeMaintenance {
        calls: Arc<Mutex<Vec<Uuid>>>,
        reports: Mutex<
            VecDeque<Result<GatewayServiceLogMaintenanceReport, GatewayServiceLogMaintenanceError>>,
        >,
        hang: bool,
    }

    #[async_trait]
    impl GatewayServiceLogMaintenance for FakeMaintenance {
        async fn maintain_project(
            &self,
            project_id: Uuid,
            _policy: GatewayServiceLogMaintenancePolicy,
        ) -> Result<GatewayServiceLogMaintenanceReport, GatewayServiceLogMaintenanceError> {
            self.calls
                .lock()
                .expect("fake maintenance lock")
                .push(project_id);
            if self.hang {
                pending().await
            }
            self.reports
                .lock()
                .expect("fake report lock")
                .pop_front()
                .unwrap_or_else(|| Ok(GatewayServiceLogMaintenanceReport::default()))
        }
    }

    fn page(
        projects: Vec<Uuid>,
        next_after: Option<Uuid>,
    ) -> GatewayServiceLogMaintenanceProjectPageResult {
        GatewayServiceLogMaintenanceProjectPageResult {
            projects,
            next_after,
        }
    }

    fn scheduler(
        projects: FakeProjects,
        maintenance: FakeMaintenance,
    ) -> GatewayServiceLogMaintenanceScheduler {
        GatewayServiceLogMaintenanceScheduler::new(
            Arc::new(projects),
            Arc::new(maintenance),
            GatewayServiceLogMaintenancePolicy::default(),
        )
    }

    #[test]
    fn rejects_invalid_pages_and_accepts_empty_terminal_after_cursor() {
        let cursor = Uuid::from_u128(1);
        let empty = page(Vec::new(), None);
        assert!(validate_page(Some(cursor), &empty).is_ok());
        assert!(validate_page(None, &page(vec![Uuid::nil()], None)).is_err());
        assert!(validate_page(Some(cursor), &page(vec![cursor], None)).is_err());
        assert!(
            validate_page(
                None,
                &page(vec![Uuid::from_u128(2), Uuid::from_u128(1)], None),
            )
            .is_err()
        );
        assert!(
            validate_page(
                None,
                &page(vec![Uuid::from_u128(1)], Some(Uuid::from_u128(2)))
            )
            .is_err()
        );
        let oversized = (0..=128).map(|value| Uuid::from_u128(value + 1)).collect();
        assert!(validate_page(None, &page(oversized, None)).is_err());
    }

    #[tokio::test]
    async fn invalid_page_retries_same_cursor_then_recovers() {
        let afters = Arc::new(Mutex::new(Vec::new()));
        let scheduler = scheduler(
            FakeProjects {
                afters: Arc::clone(&afters),
                pages: Mutex::new(VecDeque::from([
                    Ok(page(vec![Uuid::from_u128(2), Uuid::from_u128(1)], None)),
                    Ok(page(vec![Uuid::from_u128(2)], None)),
                ])),
                hang: false,
            },
            FakeMaintenance {
                calls: Arc::new(Mutex::new(Vec::new())),
                reports: Mutex::new(VecDeque::new()),
                hang: false,
            },
        );
        let report = scheduler
            .run_sweep(&CancellationToken::new())
            .await
            .expect("retrying invalid page should remain supervised")
            .expect("not cancelled");
        assert_eq!(report.projects, 1);
        assert_eq!(
            afters.lock().expect("cursor lock").as_slice(),
            &[None, None]
        );
    }

    #[tokio::test]
    async fn enumerates_129_projects_in_bounded_pages() {
        let ids: Vec<_> = (1..=129).map(Uuid::from_u128).collect();
        let first = ids[..128].to_vec();
        let second = ids[128..].to_vec();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let scheduler = scheduler(
            FakeProjects {
                afters: Arc::new(Mutex::new(Vec::new())),
                pages: Mutex::new(VecDeque::from([
                    Ok(page(first, Some(ids[127]))),
                    Ok(page(second, None)),
                ])),
                hang: false,
            },
            FakeMaintenance {
                calls: Arc::clone(&calls),
                reports: Mutex::new(VecDeque::new()),
                hang: false,
            },
        );
        let cancel = CancellationToken::new();
        let report = scheduler
            .run_sweep(&cancel)
            .await
            .expect("valid sweep")
            .expect("not cancelled");
        assert_eq!(report.pages, 2);
        assert_eq!(report.projects, 129);
        assert_eq!(calls.lock().expect("calls lock").len(), 129);
    }

    #[tokio::test]
    async fn project_failure_does_not_block_fair_cursor_progress() {
        let ids = vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)];
        let calls = Arc::new(Mutex::new(Vec::new()));
        let scheduler = scheduler(
            FakeProjects {
                afters: Arc::new(Mutex::new(Vec::new())),
                pages: Mutex::new(VecDeque::from([Ok(page(ids.clone(), None))])),
                hang: false,
            },
            FakeMaintenance {
                calls: Arc::clone(&calls),
                reports: Mutex::new(VecDeque::from([
                    Err(GatewayServiceLogMaintenanceError::Unavailable),
                    Ok(GatewayServiceLogMaintenanceReport::default()),
                    Ok(GatewayServiceLogMaintenanceReport::default()),
                ])),
                hang: false,
            },
        );
        let report = scheduler
            .run_sweep(&CancellationToken::new())
            .await
            .expect("valid sweep")
            .expect("not cancelled");
        assert_eq!(report.maintenance_failures, 1);
        assert_eq!(calls.lock().expect("calls lock").len(), 3);
    }

    #[tokio::test]
    async fn aggregates_has_more_once_per_sweep() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let scheduler = scheduler(
            FakeProjects {
                afters: Arc::new(Mutex::new(Vec::new())),
                pages: Mutex::new(VecDeque::from([Ok(page(
                    vec![Uuid::from_u128(1), Uuid::from_u128(2)],
                    None,
                ))])),
                hang: false,
            },
            FakeMaintenance {
                calls,
                reports: Mutex::new(VecDeque::from([
                    Ok(GatewayServiceLogMaintenanceReport {
                        has_more: true,
                        ..Default::default()
                    }),
                    Ok(GatewayServiceLogMaintenanceReport {
                        has_more: true,
                        ..Default::default()
                    }),
                ])),
                hang: false,
            },
        );
        let report = scheduler
            .run_sweep(&CancellationToken::new())
            .await
            .expect("valid sweep")
            .expect("not cancelled");
        assert!(report.has_more);
    }

    #[tokio::test]
    async fn enumeration_timeout_retries_and_caps_backoff() {
        let scheduler = scheduler(
            FakeProjects {
                afters: Arc::new(Mutex::new(Vec::new())),
                pages: Mutex::new(VecDeque::new()),
                hang: true,
            },
            FakeMaintenance {
                calls: Arc::new(Mutex::new(Vec::new())),
                reports: Mutex::new(VecDeque::new()),
                hang: false,
            },
        );
        let cancel = CancellationToken::new();
        let sweep = Box::pin(scheduler.run_sweep(&cancel));
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        cancel.cancel();
        assert!(sweep.await.expect("timeout sweep").is_none());
        assert_eq!(
            next_enumeration_backoff(Duration::from_secs(60)),
            Duration::from_secs(60)
        );
    }

    #[tokio::test]
    async fn project_timeout_isolated_and_cancellation_stops_both_ports() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let isolated_scheduler = scheduler(
            FakeProjects {
                afters: Arc::new(Mutex::new(Vec::new())),
                pages: Mutex::new(VecDeque::from([Ok(page(vec![Uuid::from_u128(1)], None))])),
                hang: false,
            },
            FakeMaintenance {
                calls: Arc::clone(&calls),
                reports: Mutex::new(VecDeque::new()),
                hang: true,
            },
        );
        let report = isolated_scheduler
            .run_sweep(&CancellationToken::new())
            .await
            .expect("valid sweep")
            .expect("not cancelled");
        assert_eq!(report.maintenance_failures, 1);
        assert_eq!(calls.lock().expect("calls lock").len(), 1);

        let scheduler = scheduler(
            FakeProjects {
                afters: Arc::new(Mutex::new(Vec::new())),
                pages: Mutex::new(VecDeque::new()),
                hang: true,
            },
            FakeMaintenance {
                calls: Arc::new(Mutex::new(Vec::new())),
                reports: Mutex::new(VecDeque::new()),
                hang: true,
            },
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(
            scheduler
                .run_sweep(&cancel)
                .await
                .expect("cancelled sweep")
                .is_none()
        );
    }

    #[test]
    fn enumeration_backoff_caps_at_one_minute() {
        let mut delay = INITIAL_ENUMERATION_BACKOFF;
        for _ in 0..10 {
            delay = next_enumeration_backoff(delay);
        }
        assert_eq!(delay, MAX_ENUMERATION_BACKOFF);
    }
}

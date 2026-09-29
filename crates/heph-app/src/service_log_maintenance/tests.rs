//! Scheduler unit tests.

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
            pending::<()>().await;
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
            pending::<()>().await;
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

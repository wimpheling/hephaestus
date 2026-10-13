use run_domain::RunState;
use run_orchestrator::RunOrchestrator;
use runtime_types::{LeaseId, RunId, VolumeId};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use super::super::super::support::lock;
use super::{Authorizer, Factory, Fixture, Store, orchestrator, ports::hanging_provider};

pub async fn wait_running(fixture: &Fixture) {
    while fixture.runs.run.lock().await.state != RunState::Running {
        tokio::task::yield_now().await;
    }
}

pub fn start(
    orchestrator: Arc<RunOrchestrator>,
    fixture: &Fixture,
) -> tokio::task::JoinHandle<Result<run_domain::Run, run_orchestrator::OrchestratorError>> {
    let command = fixture.command.clone();
    tokio::spawn(async move { orchestrator.start_run(&command).await })
}

#[tokio::test(start_paused = true)]
async fn empty_set_is_refreshed_every_second_and_source_withdrawal_stops_guest() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 0));
    let authorizer = Arc::new(Authorizer::default());
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store.clone(),
        authorizer.clone(),
        hanging_provider(&fixture),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    wait_running(&fixture).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    while store.heartbeats.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleanedUp);
    assert_eq!(fixture.count("destroy"), 1);
    assert!(fixture.cleanup.state.lock().await.receipt.is_some());
}

#[tokio::test(start_paused = true)]
async fn one_combined_two_second_timeout_holds_all_fences_until_physical_confirmation() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    let authorizer = Arc::new(Authorizer::default());
    fixture.provider.fail_cleanup.store(true, Ordering::SeqCst);
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store,
        authorizer.clone(),
        hanging_provider(&fixture),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    wait_running(&fixture).await;
    authorizer.hung.store(true, Ordering::SeqCst);
    let checks = authorizer.checks.load(Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    while authorizer.checks.load(Ordering::SeqCst) == checks {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(!running.is_finished());
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("finish-all"), 0);
}

#[tokio::test(start_paused = true)]
async fn changed_fence_refresh_is_rejected_without_releasing_another_run() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store.clone(),
        Arc::new(Authorizer::default()),
        hanging_provider(&fixture),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    wait_running(&fixture).await;
    let mut foreign = fixture.volumes.lease.clone();
    foreign.id = LeaseId::new();
    foreign.run_id = RunId::new();
    foreign.volume_id = VolumeId::new();
    lock(&fixture.volumes.stale).push(foreign.clone());
    store.changed_fence.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleanedUp);
    assert_eq!(*lock(&fixture.volumes.stale), vec![foreign]);
    assert_eq!(
        fixture
            .cleanup
            .state
            .lock()
            .await
            .target
            .as_ref()
            .unwrap()
            .leases()
            .len(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn live_mount_grant_denial_holds_fences_when_scoped_confirmation_fails() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    fixture.provider.fail_cleanup.store(true, Ordering::SeqCst);
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store.clone(),
        Arc::new(Authorizer::default()),
        hanging_provider(&fixture),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    wait_running(&fixture).await;
    store.mount_denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("destroy"), 1);
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("finish-all"), 0);
}

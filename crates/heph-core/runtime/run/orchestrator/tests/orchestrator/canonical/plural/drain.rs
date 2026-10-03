use super::super::super::support::lock;
use super::{Authorizer, Factory, Fixture, Store, monitor::start, orchestrator};
use run_domain::{RunOutcome, RunState};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

async fn wait_closed(fixture: &Fixture) {
    while fixture.cleanup.state.lock().await.target.is_none() {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn slow_positive_scoped_cleanup_keeps_success_and_live_checks_after_owned_closure() {
    let fixture = Fixture::new();
    fixture
        .provider
        .cleanup_delay_seconds
        .store(3, Ordering::SeqCst);
    let store = Arc::new(Store::new(&fixture, 2));
    let authorizer = Arc::new(Authorizer::default());
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store.clone(),
        authorizer.clone(),
        fixture.provider.clone(),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    wait_closed(&fixture).await;
    let checks = authorizer.checks.load(Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    while authorizer.checks.load(Ordering::SeqCst) == checks {
        tokio::task::yield_now().await;
    }
    assert!(!running.is_finished());
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(
        store.heartbeats.load(Ordering::SeqCst),
        0,
        "owned closure must not renew leases"
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    let result = running.await.unwrap().unwrap();
    assert_eq!(result.state, RunState::CleanedUp);
    assert_eq!(result.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(fixture.count("destroy"), 1);
    assert_eq!(fixture.count("scoped-cleanup"), 1);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test(start_paused = true)]
async fn caller_withdrawal_remains_terminal_during_own_positive_drain() {
    let fixture = Fixture::new();
    fixture
        .provider
        .cleanup_delay_seconds
        .store(3, Ordering::SeqCst);
    let store = Arc::new(Store::new(&fixture, 2));
    let authorizer = Arc::new(Authorizer::default());
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store,
        authorizer.clone(),
        fixture.provider.clone(),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    wait_closed(&fixture).await;
    authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(running.await.unwrap().is_err());
    assert_eq!(
        fixture.runs.run.lock().await.outcome,
        Some(RunOutcome::Failed)
    );
    assert!(fixture.cleanup.state.lock().await.receipt.is_some());
    assert!(lock(&fixture.volumes.stale).is_empty());
}

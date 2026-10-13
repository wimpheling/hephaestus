use super::super::super::support::{RecordingAuthorityManager, lock};
use super::{
    Authorizer, Factory, Fixture, Store,
    monitor::{start, wait_running},
    orchestrator,
    ports::hanging_provider,
};
use run_domain::RunState;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

#[tokio::test(start_paused = true)]
async fn stalled_event_write_cannot_starve_live_revocation() {
    let fixture = Fixture::new();
    fixture.runs.hang_events.store(true, Ordering::SeqCst);
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
    fixture.runs.event_entered.notified().await;
    authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("destroy"), 1);
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleanedUp);
    assert!(fixture.cleanup.state.lock().await.receipt.is_some());
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test(start_paused = true)]
async fn missing_acknowledgement_cannot_starve_zero_volume_caller_withdrawal() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 0));
    let authorizer = Arc::new(Authorizer::default());
    let orchestrator = Arc::new(
        orchestrator(
            &fixture,
            store,
            authorizer.clone(),
            hanging_provider(&fixture),
            Arc::new(Factory),
        )
        .with_authority_manager(Arc::new(RecordingAuthorityManager {
            log: fixture.log.clone(),
            reject_acknowledgement: false,
        })),
    );
    let running = start(orchestrator, &fixture);
    while fixture.runs.run.lock().await.state != RunState::Starting {
        tokio::task::yield_now().await;
    }
    authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("destroy"), 1);
    assert_eq!(fixture.count("authority-ack"), 0);
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleanedUp);
}

#[tokio::test(start_paused = true)]
async fn database_stall_after_start_still_destroys_guest_before_bookkeeping() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store,
        Arc::new(Authorizer::default()),
        hanging_provider(&fixture),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    wait_running(&fixture).await;
    fixture.runs.hang_get.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    // Let the heartbeat enter its combined deadline before advancing it.
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(2)).await;
    while fixture.count("destroy") == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("finish-all"), 0);
    tokio::time::advance(Duration::from_secs(4)).await;
    assert!(running.await.unwrap().is_err());
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
}

#[tokio::test(start_paused = true)]
async fn denial_during_provider_entry_quiesces_cancelled_future_before_absence() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    let authorizer = Arc::new(Authorizer::default());
    let pause = super::super::provider::Pause::new();
    *lock(&fixture.provider.pause_provision) = Some(pause.clone());
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store,
        authorizer.clone(),
        fixture.provider.clone(),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    pause.entered.notified().await;
    authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    pause.release.notify_one();
    tokio::task::yield_now().await;
    assert_eq!(
        fixture.count("provision"),
        0,
        "cancelled fake entry must never resume host IO"
    );
    assert_eq!(fixture.count("start"), 0);
    assert!(fixture.cleanup.state.lock().await.receipt.is_some());
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test(start_paused = true)]
async fn blocked_start_rpc_does_not_delay_revocation_or_resume_after_cleanup() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    let authorizer = Arc::new(Authorizer::default());
    let pause = super::super::provider::Pause::new();
    *lock(&fixture.provider.pause_start) = Some(pause.clone());
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store,
        authorizer.clone(),
        fixture.provider.clone(),
        Arc::new(Factory),
    ));
    let running = start(orchestrator, &fixture);
    pause.entered.notified().await;
    authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    pause.release.notify_one();
    tokio::task::yield_now().await;
    assert_eq!(
        fixture.count("start"),
        0,
        "cancelled Start RPC must not resume"
    );
    assert_eq!(fixture.count("destroy"), 1);
    assert!(fixture.cleanup.state.lock().await.receipt.is_some());
    assert!(lock(&fixture.volumes.stale).is_empty());
}

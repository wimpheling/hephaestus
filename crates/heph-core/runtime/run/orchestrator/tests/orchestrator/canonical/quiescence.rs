use super::super::support::{TransitionPause, lock};
use super::{Fixture, provider::Pause};
use run_domain::RunState;
use run_orchestrator::OrchestratorError;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn duplicate_start_is_denied_before_preparation_without_cleaning_first_worker() {
    let fixture = Fixture::new();
    let pause = Pause::new();
    *lock(&fixture.cleanup.pause_planning) = Some((2, pause.clone()));
    let orchestrator = Arc::new(fixture.orchestrator());
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    pause.entered.notified().await;
    let prepared = fixture.count("runtime-prepare");
    assert!(
        matches!(orchestrator.start_run(&fixture.command).await, Err(OrchestratorError::RunInProgress(id)) if id == fixture.command.run_id)
    );
    assert_eq!(fixture.count("runtime-prepare"), prepared);
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert!(fixture.cleanup.state.lock().await.target.is_none());
    pause.release.notify_one();
    assert_eq!(running.await.unwrap().unwrap().state, RunState::CleanedUp);
}

#[tokio::test]
async fn closure_waits_for_worker_paused_between_final_db_check_and_provider_entry() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    let pause = Pause::new();
    *lock(&fixture.cleanup.pause_planning) = Some((2, pause.clone()));
    let orchestrator = Arc::new(fixture.orchestrator());
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    pause.entered.notified().await;
    let mut recovering = tokio::spawn({
        let orchestrator = orchestrator.clone();
        async move { orchestrator.recover_stale_leases().await }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut recovering)
            .await
            .is_err()
    );
    assert!(fixture.cleanup.state.lock().await.target.is_some());
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    pause.release.notify_one();
    assert_eq!(recovering.await.unwrap().unwrap(), 1);
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("start"), 0);
    assert_eq!(fixture.count("scoped-cleanup"), 1);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test]
async fn cleanup_winning_before_preparation_closes_future_provider_io() {
    let fixture = Fixture::new();
    let pause = Pause::new();
    *lock(&fixture.cleanup.pause_planning) = Some((1, pause.clone()));
    let orchestrator = Arc::new(fixture.orchestrator());
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    pause.entered.notified().await;
    assert_eq!(orchestrator.recover_after_restart().await.unwrap(), 1);
    pause.release.notify_one();
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("provision"), 0);
    assert_eq!(fixture.count("start"), 0);
    assert_eq!(fixture.count("scoped-cleanup"), 1);
}

#[tokio::test]
async fn blocked_spawner_times_out_cleanup_without_receipt_or_partial_release() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    let pause = Pause::new();
    *lock(&fixture.provider.pause_provision) = Some(pause.clone());
    let orchestrator = Arc::new(
        fixture
            .orchestrator()
            .with_cleanup_timeout(Duration::from_millis(10)),
    );
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    pause.entered.notified().await;
    let error = orchestrator.recover_stale_leases().await.unwrap_err();
    assert!(error.to_string().contains("deadline elapsed"));
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert_eq!(fixture.count("completion"), 0);
    pause.release.notify_one();
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("start"), 0);
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleanedUp);
}

#[tokio::test]
async fn cleanup_waits_already_inflight_start_before_exact_destruction() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    let pause = Pause::new();
    *lock(&fixture.provider.pause_start) = Some(pause.clone());
    let orchestrator = Arc::new(fixture.orchestrator());
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    pause.entered.notified().await;
    let mut recovering = tokio::spawn({
        let orchestrator = orchestrator.clone();
        async move { orchestrator.recover_stale_leases().await }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut recovering)
            .await
            .is_err()
    );
    assert_eq!(fixture.count("destroy"), 0);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    pause.release.notify_one();
    assert_eq!(recovering.await.unwrap().unwrap(), 1);
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("destroy"), 1);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test]
async fn cancelled_start_future_drops_admission_token_without_inventing_absence() {
    let fixture = Fixture::new();
    let pause = Pause::new();
    *lock(&fixture.cleanup.pause_planning) = Some((1, pause.clone()));
    let orchestrator = Arc::new(fixture.orchestrator());
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    pause.entered.notified().await;
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert_eq!(
        orchestrator
            .start_run(&fixture.command)
            .await
            .unwrap()
            .state,
        RunState::CleanedUp
    );
}

#[tokio::test]
async fn start_failure_after_completed_cleanup_does_not_restore_destroyed_handle() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    *fixture.runs.pause_running_transition.lock().await = Some(TransitionPause {
        entered: entered.clone(),
        release: release.clone(),
    });
    let orchestrator = Arc::new(fixture.orchestrator());
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    entered.notified().await;
    assert_eq!(orchestrator.recover_stale_leases().await.unwrap(), 1);
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleanedUp);
    release.notify_one();
    assert!(running.await.unwrap().is_err());
    assert!(
        lock(&fixture.provider.instances)
            .iter()
            .all(|handle| handle.upgrade().is_none()),
        "confirmed-removed handle must not be restored by the delayed Start failure"
    );
    assert_eq!(fixture.count("destroy"), 1);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

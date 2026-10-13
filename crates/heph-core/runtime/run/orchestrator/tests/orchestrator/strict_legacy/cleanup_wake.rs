//! Controlled owning callbacks only; no PostgreSQL/physical VM proof.
use super::{
    super::{canonical::provider::Pause, support::lock},
    fixture::Fixture,
};
use run_domain::{CancelRun, RunState};
use run_orchestrator::{
    OrchestratorError, RepositoryError, RunCleanupWakeBinding, RunCleanupWakeOriginal,
    RunCleanupWakeProgress, RunCleanupWakeRequest, RunRepository,
};
use std::{
    future::Future,
    sync::{Arc, atomic::Ordering},
    task::Poll,
    time::Duration,
};
use time::OffsetDateTime;
use uuid::Uuid;
use volume_trait::ScalarLeaseHistory;

fn request(f: &Fixture) -> RunCleanupWakeRequest {
    RunCleanupWakeRequest::new(
        f.command.run_id,
        f.command.instance_id,
        RunCleanupWakeOriginal {
            drain_attempt: Uuid::new_v4(),
            drain_command: Uuid::new_v4(),
            removal_id: Uuid::new_v4(),
            command_key: [3; 32],
            input_hash: [4; 32],
            expected_version: 1,
            creator: Uuid::new_v4(),
            request: Uuid::new_v4(),
        },
    )
    .unwrap()
}
async fn qualify(f: &Fixture) -> RunCleanupWakeRequest {
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let request = request(f);
    let plan = f.runs.plan.lock().await.clone().unwrap();
    *f.runs.inner.wake.lock().await = Some((
        request.clone(),
        RunCleanupWakeBinding::Legacy(Box::new(plan)),
    ));
    f.runs.closed.store(true, Ordering::SeqCst);
    f.runs.inner.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
    request
}

#[tokio::test]
async fn cleanup_wake_duplicate_cancel_recovers_without_another_cancel_or_global_sweep() {
    let f = Fixture::new(true);
    f.provider.fail_cleanup.store(true, Ordering::SeqCst);
    let orch = Arc::new(f.orchestrator());
    assert!(orch.start_run(&f.command).await.is_err());
    let cancel = CancelRun {
        command_id: runtime_types::CommandId::new(),
        run_id: f.command.run_id,
        reason: "controlled removal".into(),
    };
    assert!(orch.cancel_run(&cancel).await.is_err());
    f.runs.cancel_replay.store(true, Ordering::SeqCst);
    assert!(!orch.cancel_run(&cancel).await.unwrap());
    let request = request(&f);
    let plan = f.runs.plan.lock().await.clone().unwrap();
    *f.runs.inner.wake.lock().await = Some((
        request.clone(),
        RunCleanupWakeBinding::Legacy(Box::new(plan)),
    ));
    f.provider.fail_cleanup.store(false, Ordering::SeqCst);
    assert_eq!(
        orch.clone().wake_cleanup(&request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(f.count("release"), 1);
    assert_eq!(f.count("completion"), 1);
    let physical = f.count("scoped-cleanup");
    assert_eq!(
        orch.wake_cleanup(&request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(f.count("scoped-cleanup"), physical);
    assert_eq!(f.count("release"), 1);
}

#[tokio::test]
async fn cleanup_wake_paused_start_or_provision_holds_without_destroying_live_execution() {
    for at_start in [false, true] {
        let f = Fixture::new(false);
        let pause = Pause::new();
        if at_start {
            *lock(&f.provider.pause_start) = Some(pause.clone());
        } else {
            *lock(&f.provider.pause_provision) = Some(pause.clone());
        }
        let orch = Arc::new(f.orchestrator());
        let start = {
            let orch = orch.clone();
            let command = f.command.clone();
            tokio::spawn(async move { orch.start_run(&command).await })
        };
        pause.entered.notified().await;
        let request = request(&f);
        let plan = f.runs.plan.lock().await.clone().unwrap();
        *f.runs.inner.wake.lock().await = Some((
            request.clone(),
            RunCleanupWakeBinding::Legacy(Box::new(plan)),
        ));
        f.runs.closed.store(true, Ordering::SeqCst);
        f.runs.inner.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
        assert_eq!(
            orch.clone().wake_cleanup(&request).await.unwrap(),
            RunCleanupWakeProgress::Held
        );
        assert_eq!(f.count("scoped-cleanup"), 0);
        assert_eq!(f.runs.inner.wake_prepare_calls.load(Ordering::SeqCst), 0);
        pause.release.notify_one();
        let _completion = start.await.unwrap();
        assert_eq!(
            orch.wake_cleanup(&request).await.unwrap(),
            RunCleanupWakeProgress::Complete
        );
    }
}

#[tokio::test]
async fn cleanup_wake_fresh_pin_substitution_or_db_failure_denies_before_physical_io() {
    let f = Fixture::new(false);
    let request = qualify(&f).await;
    f.runs.inner.wake_substitute.store(true, Ordering::SeqCst);
    assert!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&request)
            .await
            .is_err()
    );
    assert_eq!(f.count("scoped-cleanup"), 0);
    assert_eq!(f.count("completion"), 0);
    f.runs.inner.wake_substitute.store(false, Ordering::SeqCst);
    f.runs.inner.wake_prepare_fail.store(true, Ordering::SeqCst);
    assert!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&request)
            .await
            .is_err()
    );
    assert_eq!(f.count("scoped-cleanup"), 0);
}

#[tokio::test]
async fn cleanup_wake_wrong_original_closure_or_foreign_owner_never_adopts_history() {
    let f = Fixture::new(false);
    let request = qualify(&f).await;
    let mut original = request.original().clone();
    original.input_hash[0] ^= 1;
    let wrong =
        RunCleanupWakeRequest::new(request.run_id(), request.instance_id(), original).unwrap();
    assert!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&wrong)
            .await
            .is_err()
    );
    *lock(&f.provider.scope) =
        vm_trait::VmProviderOwnerScope::new("foreign".into(), "test".into()).unwrap();
    assert!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&request)
            .await
            .is_err()
    );
    assert_eq!(f.count("scoped-cleanup"), 0);
    assert_eq!(f.count("completion"), 0);
}

#[tokio::test(start_paused = true)]
async fn cleanup_wake_uncertain_confirmation_is_bounded_and_keeps_fence_and_handle() {
    let f = Fixture::new(true);
    f.provider.hang_cleanup.store(true, Ordering::SeqCst);
    let orch = Arc::new(
        f.orchestrator()
            .with_cleanup_timeout(Duration::from_secs(5)),
    );
    assert!(orch.start_run(&f.command).await.is_err());
    let request = request(&f);
    let plan = f.runs.plan.lock().await.clone().unwrap();
    *f.runs.inner.wake.lock().await = Some((
        request.clone(),
        RunCleanupWakeBinding::Legacy(Box::new(plan)),
    ));
    f.runs.inner.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
    let began = tokio::time::Instant::now();
    let progress = orch.clone().wake_cleanup(&request).await;
    // The existing scoped deadline and outer wake budget expire together.
    // Either timer may finish first; both preserve uncertain cleanup evidence.
    assert!(matches!(
        progress,
        Ok(RunCleanupWakeProgress::Held)
            | Err(OrchestratorError::Repository(RepositoryError::InvalidData(
                "Legacy scoped cleanup deadline elapsed"
            )))
    ));
    assert!(began.elapsed() <= Duration::from_secs(5));
    assert!(lock(&f.provider.instances)[0].upgrade().is_some());
    assert!(matches!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Held(_)
    ));
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
}

#[tokio::test]
async fn cleanup_wake_released_original_preserves_newer_fence_and_known_exit() {
    let f = Fixture::new(true);
    let request = qualify(&f).await;
    let original = f.volumes.set_held(f.command.run_id).await;
    *f.volumes.history.lock().await = ScalarLeaseHistory::Released(original.clone());
    let mut newer = original.clone();
    newer.fencing_token += 1;
    newer.run_id = runtime_types::RunId::new();
    lock(&f.volumes.inner.stale).push(newer.clone());
    let exit = vm_trait::VmExit {
        code: Some(0),
        signal: None,
    };
    {
        let mut row = f.runs.inner.run.lock().await;
        row.state = RunState::Succeeded;
        row.outcome = Some(run_domain::RunOutcome::Succeeded);
        row.exit = Some(exit.clone());
    }
    assert_eq!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&request)
            .await
            .unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Released(original)
    );
    assert_eq!(*lock(&f.volumes.inner.stale), vec![newer]);
    assert_eq!(f.runs.inner.run.lock().await.exit, Some(exit));
    assert_eq!(f.count("release"), 0);
}

#[tokio::test]
async fn cleanup_wake_caller_drop_keeps_same_controller_until_detached_completion() {
    let f = Fixture::new(false);
    let request = qualify(&f).await;
    f.provider.cleanup_delay_seconds.store(1, Ordering::SeqCst);
    let orch = Arc::new(f.orchestrator());
    let owner = Arc::downgrade(&orch);
    let call = tokio::spawn(async move { orch.wake_cleanup(&request).await });
    f.runs.inner.wake_entered.notified().await;
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert!(owner.upgrade().is_some());
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.count("completion") != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.count("completion"), 1);
    assert_eq!(f.count("scoped-cleanup"), 1);
}

#[tokio::test]
async fn cleanup_wake_preparation_callback_holds_operation_mutex_before_physical_io() {
    let f = Fixture::new(false);
    f.provider.fail_destroy.store(true, Ordering::SeqCst);
    let orch = Arc::new(f.orchestrator());
    assert!(orch.start_run(&f.command).await.is_err());
    assert!(lock(&f.provider.instances)[0].upgrade().is_some());
    f.provider.fail_destroy.store(false, Ordering::SeqCst);
    let request = request(&f);
    let plan = f.runs.plan.lock().await.clone().unwrap();
    *f.runs.inner.wake.lock().await = Some((
        request.clone(),
        RunCleanupWakeBinding::Legacy(Box::new(plan)),
    ));
    f.runs.inner.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
    let pause = super::super::support::TransitionPause {
        entered: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
    };
    *f.runs.inner.pause_wake_preparation.lock().await = Some(pause.clone());
    let wake = {
        let orch = orch.clone();
        tokio::spawn(async move { orch.wake_cleanup(&request).await })
    };
    pause.entered.notified().await;
    assert_eq!(f.runs.inner.wake_prepare_calls.load(Ordering::SeqCst), 1);
    let command = CancelRun {
        command_id: runtime_types::CommandId::new(),
        run_id: f.command.run_id,
        reason: "controlled concurrent cancellation".into(),
    };
    let mut cancel = Box::pin(orch.cancel_run(&command));
    // Poll the actual cancellation helper, rather than assume a spawned task
    // was scheduled. With preparation still paused, its physical operation
    // must wait on the same mutex. No elapsed delay is used as ordering proof.
    let progress = std::future::poll_fn(|cx| Poll::Ready(cancel.as_mut().poll(cx))).await;
    assert!(progress.is_pending());
    assert_eq!(f.count("scoped-cleanup"), 0);
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
    drop(cancel);
    pause.release.notify_one();
    assert_eq!(
        wake.await.unwrap().unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(f.count("completion"), 1);
}

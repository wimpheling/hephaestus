use super::{Controlled, Duration, Ordering, RunOutcome, RunState};
use crate::support::lock;

async fn wait_for_cleanup(controlled: &Controlled) {
    for _ in 0..4096 {
        if controlled.fixture.count("scoped-cleanup") > 0 {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("controlled physical cleanup must start");
}

#[tokio::test(start_paused = true)]
async fn invocation_observed_withdrawal_before_successful_wait_remains_primary() {
    let controlled = Controlled::new(2).await;
    let running = controlled.start();
    controlled.ready().await;
    controlled.authorizer.denied.store(true, Ordering::SeqCst);
    controlled
        .fixture
        .provider
        .cleanup_delay_seconds
        .store(3, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_cleanup(&controlled).await;
    // The authority monitor has already observed the failure; a later real
    // wait result cannot replace that owned error, even with successful code.
    controlled.provider.finish(0);
    let error = running.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("source/caller withdrawn"));
    let run = controlled.fixture.runs.run.lock().await.clone();
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert!(run.failure.unwrap().contains("source/caller withdrawn"));
}

#[tokio::test(start_paused = true)]
async fn invocation_waiting_event_drain_does_not_hide_an_observed_failure() {
    let controlled = Controlled::new(2).await;
    let running = controlled.start();
    controlled.ready().await;
    // A genuine wait may finish while durable event draining is still blocked.
    // This must remain Executing until that actual drain has completed.
    loop {
        let consumed = tokio::select! {
            biased;
            () = controlled.fixture.runs.event_entered.notified() => true,
            () = std::future::ready(()) => false,
        };
        if !consumed {
            break;
        }
    }
    controlled
        .fixture
        .runs
        .hang_events
        .store(true, Ordering::SeqCst);
    // The actual wait port emits the event immediately before returning its
    // genuine exit. No independent event can win first: that event is available
    // only once the wait future has returned, so its durable drain blocks.
    controlled
        .provider
        .emit_exit_event
        .store(true, Ordering::SeqCst);
    controlled.provider.finish(0);
    controlled.fixture.runs.event_entered.notified().await;
    controlled.authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    let error = running.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("source/caller withdrawn"));
    assert_eq!(
        controlled.fixture.runs.run.lock().await.outcome,
        Some(RunOutcome::Failed)
    );
    assert!(lock(&controlled.fixture.volumes.stale).is_empty());
}

#[tokio::test(start_paused = true)]
async fn invocation_own_post_exit_closure_survives_delayed_cleanup_without_checks_or_heartbeats() {
    let controlled = Controlled::new(2).await;
    controlled
        .fixture
        .provider
        .cleanup_delay_seconds
        .store(3, Ordering::SeqCst);
    let running = controlled.start();
    controlled.ready().await;
    controlled.provider.finish(0);
    wait_for_cleanup(&controlled).await;
    assert!(
        controlled
            .fixture
            .cleanup
            .state
            .lock()
            .await
            .target
            .is_some()
    );
    assert!(
        controlled
            .fixture
            .cleanup
            .state
            .lock()
            .await
            .receipt
            .is_none()
    );
    assert_eq!(lock(&controlled.fixture.volumes.stale).len(), 2);
    let checks = controlled.authorizer.checks.load(Ordering::SeqCst);
    let authority_checks = controlled.authority.checks.load(Ordering::SeqCst);
    let heartbeats = controlled.store.inner.heartbeats.load(Ordering::SeqCst);
    // This withdrawal follows actual wait plus drain. The fake live ports also
    // reject the real cleanup target's intentional execution closure.
    controlled.authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    assert!(!running.is_finished());
    assert_eq!(controlled.authorizer.checks.load(Ordering::SeqCst), checks);
    assert_eq!(
        controlled.authority.checks.load(Ordering::SeqCst),
        authority_checks
    );
    assert_eq!(
        controlled.store.inner.heartbeats.load(Ordering::SeqCst),
        heartbeats
    );
    assert!(
        controlled
            .fixture
            .cleanup
            .state
            .lock()
            .await
            .receipt
            .is_none()
    );
    let run = running.await.unwrap().unwrap();
    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    assert!(
        controlled
            .fixture
            .cleanup
            .state
            .lock()
            .await
            .receipt
            .is_some()
    );
    assert!(lock(&controlled.fixture.volumes.stale).is_empty());
}

#[tokio::test(start_paused = true)]
async fn invocation_actual_exit_cancels_a_blocked_execution_check_before_heartbeat() {
    let controlled = Controlled::new(2).await;
    controlled
        .fixture
        .provider
        .cleanup_delay_seconds
        .store(3, Ordering::SeqCst);
    let running = controlled.start();
    controlled.ready().await;
    let before = controlled.authorizer.checks.load(Ordering::SeqCst);
    controlled.authorizer.hung.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..4096 {
        if controlled.authorizer.checks.load(Ordering::SeqCst) > before {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(controlled.authorizer.checks.load(Ordering::SeqCst) > before);
    let heartbeats = controlled.store.inner.heartbeats.load(Ordering::SeqCst);
    controlled.provider.finish(0);
    wait_for_cleanup(&controlled).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    controlled.authorizer.hung.store(false, Ordering::SeqCst);
    let run = running.await.unwrap().unwrap();
    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(
        controlled.store.inner.heartbeats.load(Ordering::SeqCst),
        heartbeats
    );
}

#[tokio::test(start_paused = true)]
async fn invocation_failed_wait_keeps_execution_authority_checks_active() {
    let controlled = Controlled::new(2).await;
    controlled
        .fixture
        .provider
        .cleanup_delay_seconds
        .store(3, Ordering::SeqCst);
    let running = controlled.start();
    controlled.ready().await;
    controlled.provider.fail_wait.store(true, Ordering::SeqCst);
    controlled.provider.finish(0);
    wait_for_cleanup(&controlled).await;
    let before = controlled.authorizer.checks.load(Ordering::SeqCst);
    controlled.authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    assert!(controlled.authorizer.checks.load(Ordering::SeqCst) > before);
    assert_ne!(
        controlled.fixture.runs.run.lock().await.outcome,
        Some(RunOutcome::Succeeded)
    );
}

#[tokio::test(start_paused = true)]
async fn invocation_unknown_post_exit_cleanup_holds_every_fence_without_receipt() {
    let controlled = Controlled::new(2).await;
    controlled
        .fixture
        .provider
        .fail_cleanup
        .store(true, Ordering::SeqCst);
    let running = controlled.start();
    controlled.ready().await;
    controlled.provider.finish(0);
    assert!(running.await.unwrap().is_err());
    assert_eq!(
        controlled.fixture.runs.run.lock().await.state,
        RunState::Running
    );
    assert!(controlled.fixture.runs.run.lock().await.outcome.is_none());
    assert!(
        controlled
            .fixture
            .cleanup
            .state
            .lock()
            .await
            .receipt
            .is_none()
    );
    assert_eq!(lock(&controlled.fixture.volumes.stale).len(), 2);
}

struct TimeoutFactory;
#[async_trait::async_trait]
impl run_orchestrator::VmSpecFactory for TimeoutFactory {
    async fn build(&self, _: &run_domain::Run) -> Result<vm_trait::VmSpec, vm_trait::VmError> {
        panic!("Invocation scalar factory");
    }
    async fn build_with_volumes(
        &self,
        run: &run_domain::Run,
        selected: &volume_domain::RunVolumeSelections,
    ) -> Result<vm_trait::VmSpec, vm_trait::VmError> {
        let mut spec =
            run_orchestrator::VmSpecFactory::build_with_volumes(&super::Factory, run, selected)
                .await?;
        spec.labels
            .insert("hephaestus.wall-clock-timeout-seconds".into(), "2".into());
        Ok(spec)
    }
}

#[tokio::test(start_paused = true)]
async fn invocation_execution_timeout_does_not_retire_the_live_monitor() {
    let controlled = Controlled::with_factory(2, std::sync::Arc::new(TimeoutFactory)).await;
    controlled
        .fixture
        .provider
        .cleanup_delay_seconds
        .store(3, Ordering::SeqCst);
    let running = controlled.start();
    controlled.ready().await;
    controlled.authorizer.hung.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_cleanup(&controlled).await;
    // No actual wait result exists. Its timeout must leave the blocked live
    // check active; the two-second authority deadline still rejects execution.
    let error = running.await.unwrap().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("combined live volume authorization deadline elapsed")
    );
    assert_ne!(
        controlled.fixture.runs.run.lock().await.outcome,
        Some(RunOutcome::Succeeded)
    );
}

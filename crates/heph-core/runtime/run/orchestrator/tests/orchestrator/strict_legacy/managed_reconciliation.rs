//! Controlled Core ordering and outcomes, not native/SQL absence proof.
use crate::managed_reconciliation_sweeps as sweeps;

use super::fixture::Fixture;
use crate::{
    canonical::provider::Pause,
    support::{TestSpecFactory, lock},
};
use run_domain::{RunOutcome, RunState};
use run_orchestrator::{
    OrchestratorError, RunOrchestrator, RunRepository, SupervisorReconciliationCause as Cause,
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use vm_trait::{VmExit, VmProviderOwnerScope};
use volume_trait::ScalarLeaseHistory;

async fn born(fixture: &Fixture) {
    fixture
        .runs
        .create_run_with_legacy_placement(&fixture.command, &fixture.runs.scope)
        .await
        .unwrap();
}

#[tokio::test]
async fn restart_and_stop_record_the_truthful_cause_without_synthetic_cancel_or_sweeps() {
    for (cause, expected) in [
        (
            Cause::Restart,
            "supervisor restarted while resources were active",
        ),
        (
            Cause::Stop,
            "supervisor stopped while run resources were active",
        ),
    ] {
        let fixture = Fixture::new(false);
        born(&fixture).await;
        fixture.runs.inner.run.lock().await.state = RunState::Running;
        sweeps::guarded(fixture.orchestrator())
            .reconcile_managed_runs(cause)
            .await
            .unwrap();
        let run = fixture
            .runs
            .inner
            .get(fixture.command.run_id)
            .await
            .unwrap();
        assert_eq!(run.state, RunState::CleanedUp);
        assert_eq!(run.outcome, Some(RunOutcome::Failed));
        assert_eq!(run.failure.as_deref(), Some(expected));
        assert!(run.cancel_requested_at.is_none() && run.exit.is_none());
        assert_eq!(fixture.count("scoped-cleanup"), 1);
        assert_eq!(fixture.count("start"), 0);
    }
}

#[tokio::test]
async fn held_and_contradictory_inventory_reject_before_any_effect() {
    for held in [true, false] {
        let fixture = Fixture::new(false);
        born(&fixture).await;
        fixture.runs.held_inventory.store(held, Ordering::SeqCst);
        fixture.runs.bad_inventory.store(!held, Ordering::SeqCst);
        assert!(
            sweeps::guarded(fixture.orchestrator())
                .reconcile_managed_runs(Cause::Stop)
                .await
                .is_err()
        );
        assert!(lock(&fixture.log).is_empty());
        assert!(!fixture.runs.closed.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn changed_owner_and_default_scalar_are_held_before_inventory_or_effects() {
    let fixture = Fixture::new(false);
    born(&fixture).await;
    let scalar = RunOrchestrator::new(
        fixture.runs.clone(),
        fixture.volumes.clone(),
        fixture.provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    );
    assert!(
        sweeps::guarded(scalar)
            .reconcile_managed_runs(Cause::Stop)
            .await
            .is_err()
    );
    *lock(&fixture.provider.scope) =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "test".into()).unwrap();
    assert!(
        sweeps::guarded(fixture.orchestrator())
            .reconcile_managed_runs(Cause::Restart)
            .await
            .is_err()
    );
    assert!(lock(&fixture.log).is_empty());
}

#[tokio::test]
async fn positive_open_no_io_remains_redeliverable_without_cancel_or_closure() {
    let fixture = Fixture::new(false);
    born(&fixture).await;
    let before = fixture
        .runs
        .inner
        .get(fixture.command.run_id)
        .await
        .unwrap();
    sweeps::guarded(fixture.orchestrator())
        .reconcile_managed_runs(Cause::Stop)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .runs
            .inner
            .get(fixture.command.run_id)
            .await
            .unwrap(),
        before
    );
    assert!(!fixture.runs.closed.load(Ordering::SeqCst));
    assert!(lock(&fixture.log).is_empty());
}

#[tokio::test]
async fn terminal_exit_failure_and_existing_cancel_evidence_survive_stop() {
    for state in [RunState::Succeeded, RunState::Failed, RunState::Cancelled] {
        let fixture = Fixture::new(false);
        born(&fixture).await;
        let mut run = fixture.runs.inner.run.lock().await;
        run.state = state;
        run.outcome = state.outcome();
        run.exit = Some(VmExit {
            code: Some(7),
            signal: None,
        });
        run.failure = Some("original terminal detail".into());
        run.cancel_requested_at =
            (state == RunState::Cancelled).then(time::OffsetDateTime::now_utc);
        let before = run.clone();
        drop(run);
        sweeps::guarded(fixture.orchestrator())
            .reconcile_managed_runs(Cause::Stop)
            .await
            .unwrap();
        let after = fixture
            .runs
            .inner
            .get(fixture.command.run_id)
            .await
            .unwrap();
        assert_eq!(after.state, RunState::CleanedUp);
        assert_eq!(
            (
                after.outcome,
                after.exit,
                after.failure,
                after.cancel_requested_at
            ),
            (
                before.outcome,
                before.exit,
                before.failure,
                before.cancel_requested_at
            )
        );
    }
}

#[tokio::test]
async fn occupied_live_start_is_an_error_then_retry_succeeds_after_original_finishes() {
    let fixture = Fixture::new(false);
    let pause = Pause::new();
    *lock(&fixture.provider.pause_provision) = Some(pause.clone());
    let runner = Arc::new(fixture.orchestrator());
    let first = tokio::spawn({
        let runner = runner.clone();
        let command = fixture.command.clone();
        async move { runner.start_run(&command).await }
    });
    tokio::time::timeout(Duration::from_secs(2), pause.entered.notified())
        .await
        .unwrap();
    let before = lock(&fixture.log).len();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        runner.reconcile_managed_runs(Cause::Stop),
    )
    .await
    .unwrap();
    let after = lock(&fixture.log).len();
    pause.release.notify_one();
    let completed = tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, Err(OrchestratorError::RunInProgress(id)) if id == fixture.command.run_id)
    );
    assert_eq!(after, before);
    assert_eq!(completed.id, fixture.command.run_id);
    assert_eq!(completed.state, RunState::CleanedUp);
    assert_eq!(completed.outcome, Some(RunOutcome::Succeeded));
    // Original Start already confirms cleanup; terminal reconciliation must not repeat it.
    let physical = fixture.count("scoped-cleanup");
    assert!(physical > 0);
    runner.reconcile_managed_runs(Cause::Stop).await.unwrap();
    assert_eq!(fixture.count("scoped-cleanup"), physical);
    assert_eq!(
        fixture
            .runs
            .inner
            .get(fixture.command.run_id)
            .await
            .unwrap(),
        completed
    );
}

#[tokio::test]
async fn physical_failure_retains_exact_fence_and_terminal_detail_until_retry() {
    let fixture = Fixture::new(true);
    born(&fixture).await;
    let lease = fixture.volumes.set_held(fixture.command.run_id).await;
    fixture.runs.inner.run.lock().await.state = RunState::Running;
    fixture.provider.fail_cleanup.store(true, Ordering::SeqCst);
    let runner = sweeps::guarded(fixture.orchestrator());
    assert!(runner.reconcile_managed_runs(Cause::Stop).await.is_err());
    assert_eq!(
        *fixture.volumes.history.lock().await,
        ScalarLeaseHistory::Held(lease.clone())
    );
    assert_eq!(fixture.count("release"), 0);
    let held = fixture
        .runs
        .inner
        .get(fixture.command.run_id)
        .await
        .unwrap();
    assert_ne!(held.state, RunState::CleanedUp);
    fixture.provider.fail_cleanup.store(false, Ordering::SeqCst);
    runner.reconcile_managed_runs(Cause::Restart).await.unwrap();
    assert_eq!(
        *fixture.volumes.history.lock().await,
        ScalarLeaseHistory::Released(lease)
    );
    let cleaned = fixture
        .runs
        .inner
        .get(fixture.command.run_id)
        .await
        .unwrap();
    assert_eq!(cleaned.failure, held.failure);
    assert_eq!(fixture.count("release"), 1);
}

//! Controlled consumed Invocation evidence; no real SQL/VM certificate.
use crate::managed_reconciliation_sweeps as sweeps;

use super::{
    Factory,
    fixture::{configured, fixture},
};
use crate::support::lock;
use run_domain::{LegacyVmPlacementScope, RunOutcome, RunState};
use run_orchestrator::{RunRepository, SupervisorReconciliationCause as Cause};
use std::sync::{Arc, atomic::Ordering};
use vm_trait::{VmExit, VmProvider, VmProviderOwnerScope};

async fn consumed() -> super::super::super::Fixture {
    let fixture = fixture().await;
    let (host, vm) = fixture.runs.planned_vm.lock().await.clone().unwrap();
    let created = fixture
        .runs
        .create_run_with_vm_plan(&fixture.command, &host, &vm)
        .await
        .unwrap();
    assert!(!created.created);
    fixture.runs.run.lock().await.state = RunState::Running;
    fixture.cleanup.state.lock().await.invocation_inventory =
        Some(fixture.provider.owner_scope().unwrap());
    fixture
}

#[tokio::test]
async fn invocation_stop_and_restart_use_exact_consumption_and_truthful_labels_without_sweeps() {
    for (cause, expected) in [
        (
            Cause::Stop,
            "supervisor stopped while run resources were active",
        ),
        (
            Cause::Restart,
            "supervisor restarted while run resources were active",
        ),
    ] {
        let fixture = consumed().await;
        let runner = sweeps::guarded(configured(
            &fixture,
            0,
            fixture.provider.clone(),
            Arc::new(Factory),
        ));
        runner.reconcile_managed_runs(cause).await.unwrap();
        let run = fixture.runs.get(fixture.command.run_id).await.unwrap();
        assert_eq!(run.state, RunState::CleanedUp);
        assert_eq!(run.outcome, Some(RunOutcome::Failed));
        assert_eq!(run.failure.as_deref(), Some(expected));
        assert!(run.cancel_requested_at.is_none() && run.exit.is_none());
        assert_eq!(fixture.count("scoped-cleanup"), 1);
        assert_eq!(fixture.count("finish-all"), 1);
        assert_eq!(fixture.count("start"), 0);
    }
}

#[tokio::test]
async fn ordinary_canonical_missing_plural_unknown_inventory_and_conflicting_scopes_are_held() {
    let fixture = consumed().await;
    let ordinary = sweeps::guarded(fixture.orchestrator());
    assert!(ordinary.reconcile_managed_runs(Cause::Stop).await.is_err());
    let scope = fixture.provider.owner_scope().unwrap();
    let scalar_invocation = fixture
        .orchestrator()
        .with_qualified_invocation(scope.clone());
    assert!(
        scalar_invocation
            .reconcile_managed_runs(Cause::Stop)
            .await
            .is_err()
    );
    let legacy_scope = LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), scope.host_id().into())
            .unwrap(),
    )
    .unwrap();
    let conflicting = configured(&fixture, 0, fixture.provider.clone(), Arc::new(Factory))
        .with_legacy_vm_placement(legacy_scope);
    assert!(
        conflicting
            .reconcile_managed_runs(Cause::Stop)
            .await
            .is_err()
    );
    fixture.cleanup.state.lock().await.invocation_inventory = None;
    let unknown = sweeps::guarded(configured(
        &fixture,
        0,
        fixture.provider.clone(),
        Arc::new(Factory),
    ));
    assert!(unknown.reconcile_managed_runs(Cause::Stop).await.is_err());
    assert!(lock(&fixture.log).is_empty());
    assert!(fixture.cleanup.state.lock().await.target.is_none());
}

#[tokio::test]
async fn invocation_terminal_outcomes_exit_and_existing_cancel_survive_stop() {
    for state in [RunState::Succeeded, RunState::Failed, RunState::Cancelled] {
        let fixture = consumed().await;
        let mut run = fixture.runs.run.lock().await;
        run.state = state;
        run.outcome = state.outcome();
        run.exit = Some(VmExit {
            code: Some(9),
            signal: None,
        });
        run.failure = Some("original terminal cause".into());
        run.cancel_requested_at =
            (state == RunState::Cancelled).then(time::OffsetDateTime::now_utc);
        let before = run.clone();
        drop(run);
        let runner = sweeps::guarded(configured(
            &fixture,
            0,
            fixture.provider.clone(),
            Arc::new(Factory),
        ));
        runner.reconcile_managed_runs(Cause::Stop).await.unwrap();
        let after = fixture.runs.get(fixture.command.run_id).await.unwrap();
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
async fn invocation_plural_failure_preserves_both_fences_and_exact_target_then_retries() {
    let fixture = consumed().await;
    fixture.add_leases(2);
    let original = lock(&fixture.volumes.stale).clone();
    let runner = sweeps::guarded(configured(
        &fixture,
        0,
        fixture.provider.clone(),
        Arc::new(Factory),
    ));
    fixture.provider.fail_cleanup.store(true, Ordering::SeqCst);
    assert!(runner.reconcile_managed_runs(Cause::Stop).await.is_err());
    let target = fixture.cleanup.state.lock().await.target.clone().unwrap();
    assert_eq!(target.leases().len(), 2);
    assert_eq!(*lock(&fixture.volumes.stale), original);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("finish-all"), 0);
    fixture.provider.fail_cleanup.store(false, Ordering::SeqCst);
    runner.reconcile_managed_runs(Cause::Restart).await.unwrap();
    assert_eq!(
        fixture.cleanup.state.lock().await.target.as_ref(),
        Some(&target)
    );
    assert!(lock(&fixture.volumes.stale).is_empty());
    assert_eq!(fixture.count("finish-all"), 1);
    assert_eq!(fixture.count("recover-begin"), 0);
    assert_eq!(fixture.count("recover-finish"), 0);
    let cleaned = fixture.runs.get(fixture.command.run_id).await.unwrap();
    assert_eq!(
        cleaned.failure.as_deref(),
        Some("supervisor stopped while run resources were active")
    );
}

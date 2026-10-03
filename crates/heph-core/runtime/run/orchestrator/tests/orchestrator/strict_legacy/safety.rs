use super::super::support::lock;
use super::fixture::Fixture;
use run_domain::{RunOutcome, RunState};
use run_orchestrator::RunRepository;
use std::sync::atomic::Ordering;
use vm_trait::VmProviderOwnerScope;
use volume_trait::ScalarLeaseHistory;

#[tokio::test]
async fn exact_stateful_success_confirms_scope_before_transients_release_and_callback() {
    let f = Fixture::new(true);
    let orch = f.orchestrator();
    let run = orch.start_run(&f.command).await.unwrap();
    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    assert!(f.runs.closed.load(Ordering::SeqCst));
    assert!(matches!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Released(_)
    ));
    let log = lock(&f.log);
    let confirmation = log.iter().position(|x| *x == "scoped-cleanup").unwrap();
    let transient = log.iter().position(|x| *x == "runtime-destroy").unwrap();
    let release = log.iter().position(|x| *x == "release").unwrap();
    let callback = log.iter().position(|x| *x == "completion").unwrap();
    drop(log);
    assert!(confirmation < transient && transient < release && release < callback);
}

#[tokio::test]
async fn committed_lease_with_no_memory_attachment_is_released_only_after_confirmation() {
    let f = Fixture::new(true);
    f.volumes.fail_after_acquire.store(true, Ordering::SeqCst);
    let run = f.orchestrator().start_run(&f.command).await.unwrap();
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert!(matches!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Released(_)
    ));
    assert_eq!(f.count("provision"), 0);
    assert_eq!(f.count("start"), 0);
    assert_eq!(f.count("release"), 1);
    assert_eq!(f.count("scoped-cleanup"), 1);
}

#[tokio::test]
async fn destroy_failure_keeps_actual_handle_known_exit_fence_and_no_transient_callback() {
    let f = Fixture::new(true);
    f.provider.fail_destroy.store(true, Ordering::SeqCst);
    let orch = f.orchestrator();
    assert!(orch.start_run(&f.command).await.is_err());
    let run = f.runs.inner.run.lock().await.clone();
    assert_eq!(run.state, RunState::CleaningUp);
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert_eq!(run.exit.unwrap().code, Some(0));
    assert!(f.runs.closed.load(Ordering::SeqCst));
    assert!(matches!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Held(_)
    ));
    assert!(lock(&f.provider.instances)[0].upgrade().is_some());
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("runtime-destroy"), 0);
    assert_eq!(f.count("completion"), 0);
    f.provider.fail_destroy.store(false, Ordering::SeqCst);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(f.count("release"), 1);
    assert_eq!(f.count("completion"), 1);
}

#[tokio::test]
async fn scoped_postconfirmation_failure_does_not_remove_destroyed_handle_or_release() {
    let f = Fixture::new(true);
    f.provider.fail_cleanup.store(true, Ordering::SeqCst);
    let orch = f.orchestrator();
    assert!(orch.start_run(&f.command).await.is_err());
    assert!(lock(&f.provider.instances)[0].upgrade().is_some());
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
    assert_eq!(f.runs.inner.run.lock().await.state, RunState::CleaningUp);
}

#[tokio::test]
async fn changed_configured_provider_denies_before_birth_or_preparation() {
    let f = Fixture::new(false);
    *lock(&f.provider.scope) =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "test".into()).unwrap();
    assert!(f.orchestrator().start_run(&f.command).await.is_err());
    assert!(!*f.runs.inner.created.lock().await);
    assert!(lock(&f.log).is_empty());
}

#[tokio::test]
async fn closure_db_failure_quarantines_redelivery_after_exact_physical_fallback() {
    let f = Fixture::new(true);
    f.runs.fail_close.store(true, Ordering::SeqCst);
    let orch = f.orchestrator();
    assert!(orch.start_run(&f.command).await.is_err());
    assert_eq!(f.count("scoped-cleanup"), 1);
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
    let effects = lock(&f.log).len();
    assert!(f.orchestrator().start_run(&f.command).await.is_err());
    assert_eq!(lock(&f.log).len(), effects);
    f.runs.fail_close.store(false, Ordering::SeqCst);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(f.count("release"), 1);
}

#[tokio::test]
async fn close_returning_another_run_cannot_choose_its_operation_or_vm() {
    let f = Fixture::new(true);
    let orch = f.orchestrator();
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let expected = f.runs.plan.lock().await.clone().unwrap();
    let mut command = expected.command().clone();
    command.run_id = runtime_types::RunId::new();
    let other = run_domain::LegacyVmPlacement::new(
        command.clone(),
        expected.project(),
        *expected.contract_hash(),
        expected.scope().clone(),
        vm_trait::VmId(command.run_id.to_string()),
        expected.producer(),
        expected.consumption(),
    )
    .unwrap();
    *f.runs.returned_close.lock().await = Some(other);
    assert!(orch.start_run(&f.command).await.is_err());
    assert_no_contradictory_cleanup(&f);
    assert!(matches!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Held(_)
    ));
    assert!(lock(&f.provider.instances)[0].upgrade().is_some());
}

#[tokio::test]
async fn close_same_run_with_changed_project_is_held_before_physical_io() {
    let f = Fixture::new(true);
    let orch = f.orchestrator();
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let expected = f.runs.plan.lock().await.clone().unwrap();
    let changed = run_domain::LegacyVmPlacement::new(
        expected.command().clone(),
        uuid::Uuid::new_v4(),
        *expected.contract_hash(),
        expected.scope().clone(),
        expected.vm_id().clone(),
        expected.producer(),
        expected.consumption(),
    )
    .unwrap();
    *f.runs.returned_close.lock().await = Some(changed);
    assert!(orch.start_run(&f.command).await.is_err());
    assert_no_contradictory_cleanup(&f);
    assert!(matches!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Held(_)
    ));
    assert!(lock(&f.provider.instances)[0].upgrade().is_some());
}

#[tokio::test]
async fn cleanup_only_consumer_cannot_substitute_the_frozen_contract() {
    let f = Fixture::new(false);
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let expected = f.runs.plan.lock().await.clone().unwrap();
    let unconsumed = run_domain::LegacyVmPlacement::new(
        expected.command().clone(),
        expected.project(),
        *expected.contract_hash(),
        expected.scope().clone(),
        expected.vm_id().clone(),
        expected.producer(),
        None,
    )
    .unwrap();
    *f.runs.plan.lock().await = Some(unconsumed);
    let changed = run_domain::LegacyVmPlacement::new(
        expected.command().clone(),
        expected.project(),
        [9; 32],
        expected.scope().clone(),
        expected.vm_id().clone(),
        expected.producer(),
        Some(run_domain::LegacyVmPlacementConsumption::CleanupOnly),
    )
    .unwrap();
    *f.runs.returned_consume.lock().await = Some(changed);
    f.runs.closed.store(true, Ordering::SeqCst);
    assert!(f.orchestrator().recover_after_restart().await.is_err());
    assert_no_contradictory_cleanup(&f);
    assert_eq!(f.count("provision"), 0);
    assert_eq!(f.count("start"), 0);
    assert!(f.runs.inner.run.lock().await.state != RunState::CleanedUp);
}

fn assert_no_contradictory_cleanup(f: &Fixture) {
    assert_eq!(f.count("destroy"), 0);
    assert_eq!(f.count("scoped-cleanup"), 0);
    assert_eq!(f.count("runtime-destroy"), 0);
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
}

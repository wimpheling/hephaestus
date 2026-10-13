//! Controlled core exclusions/targets only; no genuine SQL or native VM proof.
use super::super::support::TransitionPause;
use super::{
    Fixture, invocation_preparation_repository::Repository,
    invocation_recovery::inventory::Inventory, volumes::GlobalVolumes,
};
use run_domain::{CancelRun, RunKind, RunOutcome, RunState};
use run_orchestrator::OrchestratorError;
use runtime_types::{CommandId, RunId};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio::sync::Notify;
use vm_trait::{VmProvider, VmProviderOwnerScope};

async fn queued() -> (Fixture, Arc<Repository>, CancelRun) {
    let f = Fixture::new();
    f.prepare_recovery().await;
    {
        let mut run = f.runs.run.lock().await;
        run.kind = RunKind::Invocation;
        run.attachment_id = None;
        run.state = RunState::Queued;
    }
    let command = CancelRun {
        command_id: CommandId::new(),
        run_id: f.command.run_id,
        reason: "controlled admitted cancellation".into(),
    };
    let repository = Repository::new(&f, true);
    (f, repository, command)
}

#[tokio::test]
async fn accepted_replay_prepares_before_hydration_and_retries_actual_cleanup_without_failure_outcome()
 {
    let (f, repository, command) = queued().await;
    let orchestrator = repository.orchestrator(&f);
    repository.fail_preparation.store(true, Ordering::SeqCst);
    assert!(orchestrator.cancel_run(&command).await.is_err());
    let cancellation = f.runs.run.lock().await.cancel_requested_at;
    assert!(cancellation.is_some());
    assert!(f.cleanup.state.lock().await.target.is_none());
    assert_eq!(f.count("scoped-cleanup"), 0);
    repository.fail_preparation.store(false, Ordering::SeqCst);
    f.provider.fail_cleanup.store(true, Ordering::SeqCst);
    assert!(orchestrator.cancel_run(&command).await.is_err());
    let target = f.cleanup.state.lock().await.target.clone().unwrap();
    assert!(target.leases().is_empty());
    assert!(f.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(f.runs.run.lock().await.outcome, Some(RunOutcome::Cancelled));
    assert_eq!(f.count("finish-all"), 0);
    f.provider.fail_cleanup.store(false, Ordering::SeqCst);
    assert!(!orchestrator.cancel_run(&command).await.unwrap());
    assert_eq!(f.cleanup.state.lock().await.target.as_ref(), Some(&target));
    assert_eq!(f.runs.run.lock().await.state, RunState::CleanedUp);
    assert_eq!(f.runs.run.lock().await.cancel_requested_at, cancellation);
    let physical = f.count("scoped-cleanup");
    let mut newer = f.volumes.lease.clone();
    newer.run_id = RunId::new();
    newer.fencing_token += 1;
    super::super::support::lock(&f.volumes.stale).push(newer.clone());
    assert!(!orchestrator.cancel_run(&command).await.unwrap());
    assert_eq!(*super::super::support::lock(&f.volumes.stale), vec![newer]);
    assert_eq!(f.count("scoped-cleanup"), physical);
    assert_eq!(f.count("finish-all"), 1);
    assert!(f.runs.run.lock().await.failure.is_none());
    drop(orchestrator);
}

#[tokio::test]
async fn queued_cancel_cannot_steal_live_claim_or_ack_a_held_second_delivery() {
    let (f, repository, command) = queued().await;
    let pause = TransitionPause {
        entered: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
    };
    *repository.pause_preparation.lock().await = Some(pause.clone());
    let orchestrator = Arc::new(repository.orchestrator(&f));
    let first = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = command.clone();
        async move { orchestrator.cancel_run(&command).await }
    });
    tokio::time::timeout(Duration::from_secs(2), pause.entered.notified())
        .await
        .unwrap();
    let second = CancelRun {
        command_id: CommandId::new(),
        ..command.clone()
    };
    let result = tokio::time::timeout(Duration::from_secs(2), orchestrator.cancel_run(&second))
        .await
        .unwrap();
    assert!(matches!(result, Err(OrchestratorError::RunInProgress(run)) if run == command.run_id));
    assert_eq!(repository.preparations.load(Ordering::SeqCst), 1);
    assert!(f.cleanup.state.lock().await.target.is_none());
    assert!(!first.is_finished());
    *repository.pause_preparation.lock().await = None;
    pause.release.notify_one();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), first)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    );
    let physical = f.count("scoped-cleanup");
    assert!(!orchestrator.cancel_run(&second).await.unwrap());
    assert_eq!(f.count("scoped-cleanup"), physical);
    assert_eq!(f.count("finish-all"), 1);
    drop(orchestrator);
}

#[tokio::test]
async fn default_classifier_unknown_original_and_changed_owner_deny_before_preparation_or_io() {
    let (f, repository, command) = queued().await;
    let default = f
        .orchestrator()
        .with_qualified_invocation(f.provider.owner_scope().unwrap());
    assert!(default.cancel_run(&command).await.is_err());
    drop(default);
    let orchestrator = repository.orchestrator(&f);
    repository.unknown.store(true, Ordering::SeqCst);
    assert!(orchestrator.cancel_run(&command).await.is_err());
    repository.unknown.store(false, Ordering::SeqCst);
    *super::super::support::lock(&f.provider.scope) =
        VmProviderOwnerScope::new("different-owner".into(), "test".into()).unwrap();
    assert!(orchestrator.cancel_run(&command).await.is_err());
    assert_eq!(repository.preparations.load(Ordering::SeqCst), 0);
    assert!(f.cleanup.state.lock().await.target.is_none());
    assert_eq!(f.count("scoped-cleanup"), 0);
    drop(orchestrator);
}

#[tokio::test]
async fn substituted_prepared_pins_and_nonzero_queued_target_never_release_any_fence() {
    for nonzero_target in [false, true] {
        let (f, repository, command) = queued().await;
        if nonzero_target {
            f.add_leases(2);
        } else {
            repository.substitute_revision.store(true, Ordering::SeqCst);
        }
        let before = super::super::support::lock(&f.volumes.stale).clone();
        let orchestrator = repository.orchestrator(&f);
        assert!(orchestrator.cancel_run(&command).await.is_err());
        assert_eq!(*super::super::support::lock(&f.volumes.stale), before);
        assert!(f.cleanup.state.lock().await.receipt.is_none());
        assert_eq!(f.runs.run.lock().await.state, RunState::Queued);
        assert_eq!(f.count("scoped-cleanup"), 0);
        assert_eq!(f.count("finish-all"), 0);
        drop(orchestrator);
    }
}

#[tokio::test]
async fn full_inventory_error_after_a_valid_id_precedes_every_preparation_and_effect() {
    let (f, repository, command) = queued().await;
    let inventory = Arc::new(Inventory {
        inner: f.cleanup.clone(),
        scope: f.provider.owner_scope().unwrap(),
        ids: vec![command.run_id, RunId::from_uuid(uuid::Uuid::nil())],
        cutoffs: Mutex::new(Vec::new()),
    });
    let orchestrator = repository
        .orchestrator(&f)
        .with_cleanup_repository(inventory, Arc::new(GlobalVolumes(f.volumes.clone())));
    assert!(orchestrator.recover_after_restart().await.is_err());
    assert_eq!(repository.preparations.load(Ordering::SeqCst), 0);
    assert!(f.cleanup.state.lock().await.target.is_none());
    assert_eq!(f.count("scoped-cleanup"), 0);
    drop(orchestrator);
}

#[tokio::test]
async fn stale_queued_classification_cannot_ack_or_freeze_a_fresh_consumed_original() {
    let (f, repository, command) = queued().await;
    repository
        .consumed_during_preparation
        .store(true, Ordering::SeqCst);
    let orchestrator = repository.orchestrator(&f);
    assert!(matches!(orchestrator.cancel_run(&command).await,
        Err(OrchestratorError::RunInProgress(run)) if run == command.run_id));
    assert!(f.cleanup.state.lock().await.target.is_none());
    assert_eq!(f.count("scoped-cleanup"), 0);
    let preparations = repository.preparations.load(Ordering::SeqCst);
    // The next positively consumed delivery follows Start-owned cleanup semantics,
    // without entering mutable queued preparation or claiming physical completion.
    assert!(!orchestrator.cancel_run(&command).await.unwrap());
    assert_eq!(repository.preparations.load(Ordering::SeqCst), preparations);
    assert!(f.cleanup.state.lock().await.target.is_none());
    assert_eq!(f.count("finish-all"), 0);
    drop(orchestrator);
}

//! Controlled canonical receipts only; no actual VM or `PostgreSQL` proof.
use super::{
    super::support::{TestSpecFactory, lock},
    fixture::Fixture,
};
use run_domain::RunState;
use run_orchestrator::{
    RunCleanupRepository, RunCleanupWakeBinding, RunCleanupWakeOriginal, RunCleanupWakeProgress,
    RunCleanupWakeRequest, RunOrchestrator,
};
use std::sync::{Arc, atomic::Ordering};
use time::OffsetDateTime;
use uuid::Uuid;

fn request(f: &Fixture) -> RunCleanupWakeRequest {
    RunCleanupWakeRequest::new(
        f.command.run_id,
        f.command.instance_id,
        RunCleanupWakeOriginal {
            drain_attempt: Uuid::new_v4(),
            drain_command: Uuid::new_v4(),
            removal_id: Uuid::new_v4(),
            command_key: [5; 32],
            input_hash: [6; 32],
            expected_version: 1,
            creator: Uuid::new_v4(),
            request: Uuid::new_v4(),
        },
    )
    .unwrap()
}

async fn qualify(f: &Fixture) -> RunCleanupWakeRequest {
    f.prepare_recovery().await;
    let request = request(f);
    f.runs.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
    let target = f.cleanup.begin_cleanup(f.command.run_id).await.unwrap();
    *f.runs.wake.lock().await = Some((
        request.clone(),
        RunCleanupWakeBinding::Owned {
            target: Box::new(target),
        },
    ));
    request
}

#[tokio::test]
async fn cleanup_wake_canonical_confirms_then_releases_and_replay_preserves_newer_run() {
    let f = Fixture::new();
    f.add_leases(2);
    let request = qualify(&f).await;
    let orch = Arc::new(f.orchestrator());
    assert_eq!(
        orch.clone().wake_cleanup(&request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert!(f.cleanup.state.lock().await.receipt.is_some());
    assert!(f.cleanup.state.lock().await.completed);
    assert_eq!(f.runs.run.lock().await.state, RunState::CleanedUp);
    let mut newer = f.volumes.lease.clone();
    newer.run_id = runtime_types::RunId::new();
    newer.fencing_token += 1;
    lock(&f.volumes.stale).push(newer.clone());
    let physical = f.count("scoped-cleanup");
    assert_eq!(
        orch.wake_cleanup(&request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(*lock(&f.volumes.stale), vec![newer]);
    assert_eq!(f.count("scoped-cleanup"), physical);
    assert_eq!(f.count("finish-all"), 1);
}

#[tokio::test]
async fn cleanup_wake_canonical_confirmation_failure_holds_all_fences_without_callback() {
    let f = Fixture::new();
    f.add_leases(2);
    let request = qualify(&f).await;
    f.provider.fail_cleanup.store(true, Ordering::SeqCst);
    let before = lock(&f.volumes.stale).clone();
    assert!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&request)
            .await
            .is_err()
    );
    assert_eq!(*lock(&f.volumes.stale), before);
    assert_eq!(f.count("completion"), 0);
    assert!(!f.cleanup.state.lock().await.completed);
    assert!(f.cleanup.state.lock().await.receipt.is_none());
}

#[tokio::test]
async fn cleanup_wake_bare_cleaned_status_without_original_receipt_stays_held() {
    let f = Fixture::new();
    let request = qualify(&f).await;
    f.runs.run.lock().await.state = RunState::CleanedUp;
    assert_eq!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&request)
            .await
            .unwrap(),
        RunCleanupWakeProgress::Held
    );
    assert_eq!(f.count("scoped-cleanup"), 0);
    assert_eq!(f.count("completion"), 0);
}

#[tokio::test]
async fn cleanup_wake_default_mode_and_unimplemented_qualifier_are_unsupported_before_io() {
    let f = Fixture::new();
    let request = qualify(&f).await;
    let default = RunOrchestrator::new(
        f.runs.clone(),
        f.volumes.clone(),
        f.provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    );
    assert!(Arc::new(default).wake_cleanup(&request).await.is_err());
    assert_eq!(f.runs.wake_prepare_calls.load(Ordering::SeqCst), 0);
    *f.runs.wake.lock().await = None;
    assert!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&request)
            .await
            .is_err()
    );
    assert_eq!(f.runs.wake_prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.count("scoped-cleanup"), 0);
    assert_eq!(f.count("completion"), 0);
}

#[tokio::test]
async fn cleanup_wake_changed_canonical_target_never_destroys_held_handle_or_fences() {
    for substitute_lease in [false, true] {
        let f = Fixture::new();
        f.add_leases(2);
        f.provider.fail_destroy.store(true, Ordering::SeqCst);
        let orch = Arc::new(f.orchestrator());
        assert!(orch.start_run(&f.command).await.is_err());
        assert!(lock(&f.provider.instances)[0].upgrade().is_some());
        let request = request(&f);
        f.runs.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
        let original = f.cleanup.state.lock().await.target.clone().unwrap();
        *f.runs.wake.lock().await = Some((
            request.clone(),
            RunCleanupWakeBinding::Owned {
                target: Box::new(original.clone()),
            },
        ));
        let mut leases = original.leases().to_vec();
        if substitute_lease {
            let old = &leases[0];
            leases[0] = run_domain::RunCleanupLeaseFence::new(
                old.run_id(),
                old.lease_id(),
                runtime_types::VolumeId::new(),
                old.host_id().to_owned(),
                old.fencing_token(),
            )
            .unwrap();
        }
        let replacement = run_domain::RunCleanupTarget::new(
            original.run_id(),
            original.instance_id(),
            if substitute_lease {
                original.revision_id()
            } else {
                runtime_types::AgentInstanceRevisionId::new()
            },
            original.vm_target().clone(),
            original.generation(),
            leases,
        )
        .unwrap();
        assert_ne!(replacement, original);
        assert_eq!(replacement.leases().len(), original.leases().len());
        f.cleanup.state.lock().await.target = Some(replacement);
        f.provider.fail_destroy.store(false, Ordering::SeqCst);
        let fences = lock(&f.volumes.stale).clone();
        let physical = f.count("scoped-cleanup");
        let destroyed = f.count("destroy");
        assert!(orch.clone().wake_cleanup(&request).await.is_err());
        assert_eq!(f.count("scoped-cleanup"), physical);
        assert_eq!(f.count("destroy"), destroyed);
        assert_eq!(*lock(&f.volumes.stale), fences);
        assert_eq!(f.count("finish-all"), 0);
        assert_eq!(f.count("completion"), 0);
        assert!(lock(&f.provider.instances)[0].upgrade().is_some());
    }
}

#[tokio::test]
async fn cleanup_wake_prepares_initial_missing_target_before_readonly_comparison() {
    let f = Fixture::new();
    f.add_leases(2);
    f.prepare_recovery().await;
    let request = request(&f);
    f.runs.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
    assert!(f.cleanup.state.lock().await.target.is_none());
    assert!(f.runs.wake.lock().await.is_none());
    *f.runs.wake_preparation.lock().await = Some((request.clone(), f.cleanup.clone()));
    assert_eq!(
        Arc::new(f.orchestrator())
            .wake_cleanup(&request)
            .await
            .unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(f.runs.wake_prepare_calls.load(Ordering::SeqCst), 1);
    let target = f.cleanup.state.lock().await.target.clone().unwrap();
    assert_eq!(target.leases().len(), 2);
    assert!(f.cleanup.state.lock().await.completed);
    assert_eq!(f.count("finish-all"), 1);
}

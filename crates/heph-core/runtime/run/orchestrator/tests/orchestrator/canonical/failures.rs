use run_domain::{RunCleanupVmObservationKind, RunState};
use run_orchestrator::RunOrchestrator;
use std::sync::{Arc, atomic::Ordering};

use super::super::support::{AutoExitProvider, DenyLaunchAuthorizer, TestSpecFactory, lock};
use super::Fixture;

#[tokio::test]
async fn zero_volume_failure_closes_explicit_empty_set_before_completion() {
    let fixture = Fixture::new();
    let orchestrator = fixture
        .orchestrator()
        .with_launch_authorizer(Arc::new(DenyLaunchAuthorizer));
    let run = orchestrator.start_run(&fixture.command).await.unwrap();
    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(fixture.count("provision"), 0);
    assert_eq!(fixture.count("scoped-cleanup"), 1);
    assert_eq!(
        fixture
            .cleanup
            .state
            .lock()
            .await
            .receipt
            .as_ref()
            .unwrap()
            .observation()
            .kind(),
        RunCleanupVmObservationKind::AuthoritativelyAbsent
    );
    assert!(
        fixture
            .cleanup
            .state
            .lock()
            .await
            .target
            .as_ref()
            .unwrap()
            .leases()
            .is_empty()
    );
    let log = lock(&fixture.log).clone();
    let position = |name| log.iter().position(|entry| *entry == name).unwrap();
    assert!(position("bind-planned") < position("freeze"));
    assert!(position("freeze") < position("scoped-cleanup"));
    assert!(position("receipt") < position("runtime-destroy"));
    assert!(position("runtime-destroy") < position("finish-all"));
    assert!(position("finish-all") < position("completion"));
}

#[tokio::test(start_paused = true)]
async fn physical_cleanup_timeout_holds_every_fence_and_active_handle_for_retry() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    fixture.provider.hang_cleanup.store(true, Ordering::SeqCst);
    let orchestrator = fixture
        .orchestrator()
        .with_cleanup_timeout(std::time::Duration::from_millis(10));
    let error = orchestrator.start_run(&fixture.command).await.unwrap_err();
    assert!(error.to_string().contains("deadline elapsed"));
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert_eq!(fixture.count("completion"), 0);
    fixture.provider.hang_cleanup.store(false, Ordering::SeqCst);
    assert_eq!(orchestrator.recover_stale_leases().await.unwrap(), 1);
    // The exact active handle survived timeout; retry destroys it again before
    // scoped confirmation instead of inferring absence from a missing handle.
    assert_eq!(fixture.count("destroy"), 2);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test]
async fn failed_acquire_with_no_returned_attachment_releases_entire_durable_set() {
    let mut fixture = Fixture::new();
    fixture.command.requires_state = true;
    fixture.runs.run.lock().await.requires_state = true;
    fixture.add_leases(2);
    fixture.volumes.fail_acquire.store(true, Ordering::SeqCst);
    let run = fixture
        .orchestrator()
        .start_run(&fixture.command)
        .await
        .unwrap();
    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(
        fixture
            .cleanup
            .state
            .lock()
            .await
            .target
            .as_ref()
            .unwrap()
            .leases()
            .len(),
        2
    );
    assert!(lock(&fixture.volumes.stale).is_empty());
    assert_eq!(fixture.count("release"), 0);
    assert_eq!(fixture.count("finish-all"), 1);
    assert_eq!(fixture.count("completion"), 1);
}

#[tokio::test]
async fn unsupported_owner_fails_before_any_acquisition_or_preparation() {
    let fixture = Fixture::new();
    let orchestrator = RunOrchestrator::new(
        fixture.runs.clone(),
        fixture.volumes.clone(),
        Arc::new(AutoExitProvider::new(Arc::clone(&fixture.log))),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_cleanup_repository(
        fixture.cleanup.clone(),
        Arc::new(super::volumes::GlobalVolumes(fixture.volumes.clone())),
    );
    assert!(orchestrator.start_run(&fixture.command).await.is_err());
    assert_eq!(fixture.runs.run.lock().await.state, RunState::Queued);
    assert!(fixture.cleanup.state.lock().await.target.is_none());
    assert!(lock(&fixture.log).is_empty());
}

#[tokio::test]
async fn scoped_cleanup_failure_preserves_primary_failure_and_all_fences() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    fixture.provider.fail_cleanup.store(true, Ordering::SeqCst);
    let error = fixture
        .orchestrator()
        .with_launch_authorizer(Arc::new(DenyLaunchAuthorizer))
        .start_run(&fixture.command)
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("denied by test policy"), "{message}");
    assert!(message.contains("physical cleanup uncertain"), "{message}");
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("finish-all"), 0);
    assert_eq!(fixture.count("completion"), 0);
    assert_eq!(fixture.count("scoped-cleanup"), 1);
}

#[tokio::test]
async fn active_destroy_failure_retains_fences_until_retry() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    fixture.provider.fail_destroy.store(true, Ordering::SeqCst);
    let orchestrator = fixture.orchestrator();
    assert!(orchestrator.start_run(&fixture.command).await.is_err());
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert_eq!(fixture.count("completion"), 0);
    fixture.provider.fail_destroy.store(false, Ordering::SeqCst);
    assert_eq!(orchestrator.recover_stale_leases().await.unwrap(), 1);
    assert_eq!(fixture.count("destroy"), 1);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test]
async fn mismatched_active_vm_is_never_destroyed_as_the_exact_target() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    fixture.provider.wrong_handle.store(true, Ordering::SeqCst);
    assert!(
        fixture
            .orchestrator()
            .start_run(&fixture.command)
            .await
            .is_err()
    );
    assert_eq!(fixture.count("destroy"), 0);
    assert_eq!(fixture.count("start"), 0);
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert_eq!(fixture.count("finish-all"), 0);
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
}

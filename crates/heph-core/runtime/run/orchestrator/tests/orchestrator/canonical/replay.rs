use run_domain::RunState;
use runtime_types::{LeaseId, RunId};

use super::super::support::lock;
use super::Fixture;

#[tokio::test]
async fn completed_receipt_replay_preserves_new_consumer_fence_and_skips_destroy() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    fixture
        .orchestrator()
        .start_run(&fixture.command)
        .await
        .unwrap();
    let mut new_lease = fixture.volumes.lease.clone();
    let old_fence = fixture
        .cleanup
        .state
        .lock()
        .await
        .target
        .as_ref()
        .unwrap()
        .leases()[0]
        .clone();
    new_lease.volume_id = old_fence.volume_id();
    new_lease.run_id = RunId::new();
    new_lease.id = LeaseId::new();
    new_lease.fencing_token = old_fence.fencing_token() + 1;
    lock(&fixture.volumes.stale).push(new_lease.clone());
    let run = fixture
        .orchestrator()
        .start_run(&fixture.command)
        .await
        .unwrap();
    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(fixture.count("destroy"), 1);
    assert_eq!(fixture.count("scoped-cleanup"), 1);
    assert_eq!(fixture.count("finish-all"), 1);
    assert_eq!(*lock(&fixture.volumes.stale), vec![new_lease]);
}

#[tokio::test]
async fn completed_receipt_still_rejects_wrong_provider_scope_before_replay() {
    let fixture = Fixture::new();
    fixture
        .orchestrator()
        .start_run(&fixture.command)
        .await
        .unwrap();
    *lock(&fixture.provider.scope) =
        vm_trait::VmProviderOwnerScope::new("replacement".into(), "test".into()).unwrap();
    assert!(
        fixture
            .orchestrator()
            .start_run(&fixture.command)
            .await
            .is_err()
    );
    assert_eq!(fixture.count("scoped-cleanup"), 1);
    assert_eq!(fixture.count("completion"), 1);
}

#[tokio::test]
async fn persisted_historical_cleaned_run_without_receipt_is_grandfathered_only_when_empty() {
    let fixture = Fixture::new();
    fixture.runs.run.lock().await.state = RunState::CleanedUp;
    *fixture.runs.created.lock().await = true;
    let orchestrator = fixture.orchestrator();
    assert_eq!(
        orchestrator
            .start_run(&fixture.command)
            .await
            .unwrap()
            .state,
        RunState::CleanedUp
    );
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert!(fixture.cleanup.state.lock().await.target.is_none());
    fixture.add_leases(1);
    assert!(orchestrator.start_run(&fixture.command).await.is_err());
    assert_eq!(fixture.count("completion"), 1);
    assert_eq!(lock(&fixture.volumes.stale).len(), 1);
}

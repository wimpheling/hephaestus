use run_domain::RunState;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vm_trait::VmProviderOwnerScope;

use super::super::support::lock;
use super::{Fixture, fixture::FailingTransient};

#[tokio::test]
async fn recovery_groups_all_leases_once_by_run_without_scalar_release() {
    let fixture = Fixture::new();
    fixture.prepare_recovery().await;
    fixture.add_leases(2);
    assert_eq!(
        fixture.orchestrator().recover_stale_leases().await.unwrap(),
        1
    );
    assert_eq!(fixture.count("scoped-cleanup"), 1);
    assert_eq!(fixture.count("recover-begin"), 0);
    assert_eq!(fixture.count("recover-finish"), 0);
    assert_eq!(fixture.count("finish-all"), 1);
    assert_eq!(fixture.count("completion"), 1);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test]
async fn historical_null_vm_stays_held_without_uuid_fallback() {
    let fixture = Fixture::new();
    fixture.runs.run.lock().await.state = RunState::Running;
    fixture.add_leases(2);
    assert!(
        fixture
            .orchestrator()
            .recover_after_restart()
            .await
            .is_err()
    );
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert_eq!(fixture.count("runtime-destroy"), 0);
    assert_eq!(fixture.count("completion"), 0);
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
}

#[tokio::test]
async fn historical_custom_vm_without_host_owner_remains_unresolved() {
    let fixture = Fixture::new();
    {
        let mut run = fixture.runs.run.lock().await;
        run.state = RunState::Running;
        run.vm_id = Some("historical-custom-vm".into());
    }
    fixture.add_leases(1);
    assert!(
        fixture
            .orchestrator()
            .recover_after_restart()
            .await
            .is_err()
    );
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert_eq!(fixture.count("completion"), 0);
    assert_eq!(lock(&fixture.volumes.stale).len(), 1);
}

#[tokio::test]
async fn changed_provider_cannot_destroy_or_confirm_old_owner_absence() {
    let fixture = Fixture::new();
    fixture.prepare_recovery().await;
    fixture.add_leases(2);
    *lock(&fixture.provider.scope) =
        VmProviderOwnerScope::new("replacement".into(), "test".into()).unwrap();
    assert!(fixture.orchestrator().recover_stale_leases().await.is_err());
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert_eq!(fixture.count("finish-all"), 0);
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
}

#[tokio::test]
async fn transient_failure_holds_fences_then_restart_reuses_persisted_receipt() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    let fail = Arc::new(AtomicBool::new(true));
    let orchestrator = fixture
        .orchestrator()
        .with_runtime_manager(Arc::new(FailingTransient {
            fail: Arc::clone(&fail),
            log: Arc::clone(&fixture.log),
        }));
    assert!(orchestrator.start_run(&fixture.command).await.is_err());
    assert!(fixture.cleanup.state.lock().await.receipt.is_some());
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleaningUp);
    assert_eq!(fixture.count("scoped-cleanup"), 1);
    assert_eq!(fixture.count("completion"), 0);
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    fail.store(false, Ordering::SeqCst);
    assert_eq!(
        fixture
            .orchestrator()
            .recover_after_restart()
            .await
            .unwrap(),
        1
    );
    assert_eq!(fixture.count("scoped-cleanup"), 1);
    assert_eq!(fixture.count("destroy"), 1);
    assert_eq!(fixture.count("finish-all"), 1);
    assert_eq!(fixture.count("completion"), 1);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

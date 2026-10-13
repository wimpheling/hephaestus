use super::Fixture;
use run_domain::{RunCleanupHostId, RunState};
use run_orchestrator::{RunCleanupRepository, RunRepository};
use vm_trait::VmId;

fn scope() -> RunCleanupHostId {
    RunCleanupHostId::new("test-owner".into(), "test".into()).unwrap()
}

#[tokio::test]
async fn committed_plan_supports_empty_cleanup_before_separate_binding_check() {
    let fixture = Fixture::new();
    let vm = VmId(fixture.command.run_id.to_string());
    let created = fixture
        .runs
        .create_run_with_vm_plan(&fixture.command, &scope(), &vm)
        .await
        .unwrap();
    assert!(created.created);
    assert_eq!(created.run.vm_id.as_deref(), Some(vm.0.as_str()));
    assert!(fixture.cleanup.state.lock().await.binding.is_none());
    let target = fixture
        .cleanup
        .begin_cleanup(fixture.command.run_id)
        .await
        .unwrap();
    assert!(target.leases().is_empty());
    assert_eq!(target.vm_target().host(), Some(&scope()));
    assert_eq!(target.vm_target().vm_id(), Some(&vm));
    assert_eq!(fixture.count("provision"), 0);
}

#[tokio::test]
async fn immutable_input_and_scope_mismatches_do_not_clean_existing_run() {
    let fixture = Fixture::new();
    fixture
        .runs
        .create_run_with_vm_plan(
            &fixture.command,
            &scope(),
            &VmId(fixture.command.run_id.to_string()),
        )
        .await
        .unwrap();
    let before = fixture.runs.run.lock().await.clone();
    let mut changed = fixture.command.clone();
    changed.requires_state = !changed.requires_state;
    assert!(fixture.orchestrator().start_run(&changed).await.is_err());
    *super::super::support::lock(&fixture.provider.scope) =
        vm_trait::VmProviderOwnerScope::new("another-owner".into(), "test".into()).unwrap();
    assert!(
        fixture
            .orchestrator()
            .start_run(&fixture.command)
            .await
            .is_err()
    );
    assert_eq!(*fixture.runs.run.lock().await, before);
    assert!(fixture.cleanup.state.lock().await.target.is_none());
    assert_eq!(fixture.count("provision"), 0);
    assert_eq!(fixture.count("completion"), 0);
}

#[tokio::test]
async fn historical_queued_run_is_not_adopted_by_new_atomic_path() {
    let fixture = Fixture::new();
    fixture.runs.create_run(&fixture.command).await.unwrap();
    assert!(
        fixture
            .orchestrator()
            .start_run(&fixture.command)
            .await
            .is_err()
    );
    assert_eq!(fixture.runs.run.lock().await.state, RunState::Queued);
    assert!(fixture.runs.planned_vm.lock().await.is_none());
    assert!(fixture.cleanup.state.lock().await.target.is_none());
    assert_eq!(fixture.count("runtime-prepare"), 0);
}

use run_domain::{Run, RunOutcome, RunState};
use run_orchestrator::RunCleanupRepository;
use std::sync::{Arc, atomic::Ordering};
use vm_trait::VmProvider;

use super::super::super::support::{TestSpecFactory, lock};
use super::super::provider::Pause;
use super::{Authorizer, Factory, Fixture, Store, orchestrator};

#[tokio::test]
async fn exact_zero_two_and_thirty_two_sets_use_typed_disks_without_scalar_projection() {
    for count in [0, 2, 32] {
        let fixture = Fixture::new();
        let store = Arc::new(Store::new(&fixture, count));
        let authorizer = Arc::new(Authorizer::default());
        let orchestrator = orchestrator(
            &fixture,
            store,
            authorizer.clone(),
            fixture.provider.clone(),
            Arc::new(Factory),
        );
        let result = orchestrator.start_run(&fixture.command).await.unwrap();
        let spec = fixture.provider.inner.spec().unwrap();
        assert_eq!(result.state, RunState::CleanedUp);
        assert_eq!(spec.disks.len(), count);
        assert_eq!(spec.guest_volumes.len(), count);
        assert!(result.volume_id.is_none());
        assert!(result.lease_id.is_none());
        assert!(authorizer.checks.load(Ordering::SeqCst) >= 4);
        assert!(lock(&fixture.volumes.stale).is_empty());
        assert_eq!(fixture.count("finish-all"), 1);
    }
}

#[tokio::test]
async fn incomplete_or_foreign_host_return_keeps_global_evidence_until_confirmed_cleanup() {
    for foreign in [false, true] {
        let fixture = Fixture::new();
        let store = Arc::new(Store::new(&fixture, 2));
        store.foreign.store(foreign, Ordering::SeqCst);
        store.incomplete.store(!foreign, Ordering::SeqCst);
        fixture.provider.fail_cleanup.store(true, Ordering::SeqCst);
        let orchestrator = orchestrator(
            &fixture,
            store,
            Arc::new(Authorizer::default()),
            fixture.provider.clone(),
            Arc::new(Factory),
        );
        assert!(orchestrator.start_run(&fixture.command).await.is_err());
        assert_eq!(lock(&fixture.volumes.stale).len(), 2);
        assert!(fixture.cleanup.state.lock().await.receipt.is_none());
        assert_eq!(fixture.count("provision"), 0);
        assert_eq!(fixture.count("finish-all"), 0);
    }
}

#[tokio::test]
async fn factories_and_live_authorizers_must_explicitly_support_plural_profile() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    let orchestrator = orchestrator(
        &fixture,
        store,
        Arc::new(Authorizer::default()),
        fixture.provider.clone(),
        Arc::new(TestSpecFactory),
    );
    let failed = orchestrator.start_run(&fixture.command).await.unwrap();
    assert_failed_spec(
        &fixture,
        &failed,
        "complete-set VM specification is unsupported",
        2,
    )
    .await;

    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 0));
    let orchestrator = fixture
        .orchestrator()
        .with_cleanup_repository(fixture.cleanup.clone(), store)
        .with_volume_preparation();
    assert!(orchestrator.start_run(&fixture.command).await.is_err());
    assert_eq!(fixture.count("provision"), 0);
}

#[tokio::test]
async fn closure_after_provision_rejects_start_and_drains_the_complete_set() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    let pause = Pause::new();
    *lock(&fixture.provider.pause_provision) = Some(pause.clone());
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store,
        Arc::new(Authorizer::default()),
        fixture.provider.clone(),
        Arc::new(Factory),
    ));
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    pause.entered.notified().await;
    fixture
        .cleanup
        .begin_cleanup(fixture.command.run_id)
        .await
        .unwrap();
    pause.release.notify_one();
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("start"), 0);
    assert!(lock(&fixture.volumes.stale).is_empty());
    assert_eq!(fixture.count("destroy"), 1);
}

#[tokio::test]
async fn source_withdrawal_after_provision_is_checked_immediately_before_start() {
    let fixture = Fixture::new();
    let store = Arc::new(Store::new(&fixture, 2));
    let authorizer = Arc::new(Authorizer::default());
    let pause = Pause::new();
    *lock(&fixture.provider.pause_provision) = Some(pause.clone());
    let orchestrator = Arc::new(orchestrator(
        &fixture,
        store,
        authorizer.clone(),
        fixture.provider.clone(),
        Arc::new(Factory),
    ));
    let running = tokio::spawn({
        let orchestrator = orchestrator.clone();
        let command = fixture.command.clone();
        async move { orchestrator.start_run(&command).await }
    });
    pause.entered.notified().await;
    authorizer.denied.store(true, Ordering::SeqCst);
    pause.release.notify_one();
    assert!(running.await.unwrap().is_err());
    assert_eq!(fixture.count("start"), 0);
    assert_eq!(fixture.count("destroy"), 1);
    assert!(lock(&fixture.volumes.stale).is_empty());
    assert!(fixture.provider.owner_scope().is_ok());
}

#[tokio::test]
async fn changed_released_pins_and_denied_mount_authority_reject_before_acquisition() {
    for wrong_release in [false, true] {
        let fixture = Fixture::new();
        let store = Arc::new(Store::new(&fixture, 0));
        store.wrong_release.store(wrong_release, Ordering::SeqCst);
        store.mount_denied.store(!wrong_release, Ordering::SeqCst);
        let orchestrator = orchestrator(
            &fixture,
            store,
            Arc::new(Authorizer::default()),
            fixture.provider.clone(),
            Arc::new(Factory),
        );
        assert!(orchestrator.start_run(&fixture.command).await.is_err());
        assert_eq!(fixture.count("provision"), 0);
        assert!(lock(&fixture.volumes.stale).is_empty());
    }
}

#[tokio::test]
async fn only_proven_legacy_origin_projects_the_scalar_tuple_and_builtin_state_labels() {
    let mut fixture = Fixture::new();
    fixture.command.requires_state = true;
    fixture.runs.run.lock().await.requires_state = true;
    let store = Arc::new(Store::legacy(&fixture));
    let expected = store.attachments[0].lease.lease().clone();
    let orchestrator = orchestrator(
        &fixture,
        store,
        Arc::new(Authorizer::default()),
        fixture.provider.clone(),
        Arc::new(Factory),
    );
    let result = orchestrator.start_run(&fixture.command).await.unwrap();
    assert_eq!(result.volume_id, Some(expected.volume_id));
    assert_eq!(result.lease_id, Some(expected.id));
    assert_eq!(result.lease_fencing_token, Some(expected.fencing_token));
    let spec = fixture.provider.inner.spec().unwrap();
    assert!(spec.guest_volumes.is_empty());
    assert_eq!(spec.disks[0].id, volume_trait::INSTANCE_STATE_DISK_ID);
    assert_eq!(
        spec.labels["hephaestus.agent-state.mount-path"],
        "/var/lib/hephaestus"
    );
    assert_eq!(
        fixture.count("attached"),
        0,
        "attachment confirmation must use the full-set port"
    );
}

#[tokio::test]
async fn named_legacy_declaration_cannot_fall_back_to_scalar_or_skip_initialization_purpose() {
    let fixture = Fixture::new();
    let mut store = Store::legacy(&fixture);
    let old = &store.selected.selections()[0];
    let selection = volume_domain::RunVolumeSelection::new(
        old.identity(),
        old.scope().clone(),
        old.declaration().clone(),
        volume_domain::VolumeSelectionOrigin::LegacyDeclaration,
    )
    .unwrap();
    store.selected =
        volume_domain::RunVolumeSelections::new(selection.identity(), vec![selection.clone()])
            .unwrap();
    store.attachments[0].volume.instance_id = None;
    store.attachments[0].lease = volume_trait::RunVolumeLease::selected(
        store.attachments[0].lease.lease().clone(),
        selection,
    )
    .unwrap();
    let orchestrator = orchestrator(
        &fixture,
        Arc::new(store),
        Arc::new(Authorizer::default()),
        fixture.provider.clone(),
        Arc::new(Factory),
    );
    let failed = orchestrator.start_run(&fixture.command).await.unwrap();
    assert_failed_spec(
        &fixture,
        &failed,
        "named built-in state initialization requires guest protocol 11",
        1,
    )
    .await;
    assert!(failed.volume_id.is_none());
    assert!(failed.lease_id.is_none());
}

async fn assert_failed_spec(fixture: &Fixture, run: &Run, reason: &str, leases: usize) {
    // Spec rejection is a persisted terminal run result, as in scalar preparation.
    // Ok(CleanedUp) cannot mean successful workload execution here.
    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert!(
        run.failure
            .as_deref()
            .is_some_and(|failure| failure.contains(reason))
    );
    assert_eq!(fixture.count("provision"), 0);
    assert_eq!(fixture.count("start"), 0);
    let state = fixture.cleanup.state.lock().await;
    assert!(state.completed);
    assert_eq!(
        state.receipt.as_ref().unwrap().target(),
        state.target.as_ref().unwrap()
    );
    assert_eq!(state.target.as_ref().unwrap().leases().len(), leases);
    drop(state);
    assert!(lock(&fixture.volumes.stale).is_empty());
    let log = lock(&fixture.log);
    let physical = log
        .iter()
        .position(|entry| *entry == "scoped-cleanup")
        .unwrap();
    let receipt = log.iter().position(|entry| *entry == "receipt").unwrap();
    let finish = log.iter().position(|entry| *entry == "finish-all").unwrap();
    let completion = log.iter().position(|entry| *entry == "completion").unwrap();
    let completions = log.iter().filter(|entry| **entry == "completion").count();
    drop(log);
    assert!(physical < receipt && receipt < finish && finish < completion);
    assert_eq!(completions, 1);
}

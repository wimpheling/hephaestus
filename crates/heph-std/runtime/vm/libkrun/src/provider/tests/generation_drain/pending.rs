//! Existing unchanged scenario bodies; parent retains original test names.
use super::super::support::*;
use super::{DrainWorker, Spawner, owned};
use crate::provider::VmAttemptPhase;

pub(super) async fn drain_rejects_unowned_wrong_scope_and_empty_reopen_preserves_unknown_children()
{
    let temp = TempDir::new().unwrap();
    let (config, _, runtime, cgroups) = emulated_config(&temp);
    let worker = DrainWorker::new(false, false);
    let unowned =
        LibkrunProvider::new_with_spawner(config.clone(), Spawner::new(worker.clone(), false))
            .unwrap();
    let provider = owned(&config, Spawner::new(worker.clone(), false));
    let scope = provider.owner_scope().unwrap();
    assert!(matches!(
        unowned.drain_generation(&scope).await,
        Err(VmError::Unsupported { .. })
    ));
    let other = TempDir::new().unwrap();
    let (other_config, _, _, _) = emulated_config(&other);
    let other_provider = owned(&other_config, Spawner::new(worker.clone(), false));
    assert!(
        provider
            .drain_generation(&other_provider.owner_scope().unwrap())
            .await
            .is_err()
    );
    assert!(
        !provider
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .admission_closed
    );
    drop(provider);
    fs::create_dir(runtime.join("prior-process-unknown")).unwrap();
    fs::create_dir(cgroups.join("prior-process-unknown")).unwrap();
    let sentinel = runtime.join("prior-process-unknown/sentinel");
    fs::write(&sentinel, b"original unclassified bytes").unwrap();
    let reopened = LibkrunProvider::open_owned(config.clone(), &scope).unwrap();
    reopened.drain_generation(&scope).await.unwrap();
    let inventory = reopened.attempt_inventory(&scope).await.unwrap();
    assert!(inventory.admission_closed);
    assert!(inventory.attempts.is_empty());
    assert_eq!(fs::read(sentinel).unwrap(), b"original unclassified bytes");
    assert!(cgroups.join("prior-process-unknown").is_dir());
    assert_eq!(worker.kills.load(Ordering::Relaxed), 0);
    drop(reopened);
    drop(unowned);
    drop(other_provider);
    drop(worker);
}

pub(super) async fn cancelled_drain_keeps_cleanup_owned_and_refuses_live_pending_provision() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    config.startup_timeout = Duration::from_secs(5);
    let worker = DrainWorker::new(true, false);
    let spawner = Spawner::new(worker.clone(), true);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let provisioning = tokio::spawn({
        let provider = provider.clone();
        let requested = spec("pending", root.clone());
        async move { provider.provision(requested).await }
    });
    spawner.entered.notified().await;
    let draining = tokio::spawn({
        let provider = provider.clone();
        let scope = scope.clone();
        async move { provider.drain_generation(&scope).await }
    });
    worker.kill_entered.notified().await;
    draining.abort();
    assert!(draining.await.unwrap_err().is_cancelled());
    assert!(!provisioning.is_finished());
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("pending").is_dir());
    assert!(cgroups.join("pending").is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&VmId("pending".into()))
    );
    assert!(provider.provision(spec("late", root)).await.is_err());
    assert_eq!(spawner.calls.load(Ordering::Relaxed), 1);
    assert!(LibkrunProvider::open_owned(config.clone(), &scope).is_err());
    worker.release_kill.notify_one();
    let error = provisioning.await.unwrap().err().unwrap();
    assert!(error.to_string().contains("generation drain requested"));
    // A second owning drain joins/serializes behind the canceled caller's task.
    provider.drain_generation(&scope).await.unwrap();
    assert_cancelled_drain_completed(&provider, &scope, &worker, &runtime, &cgroups).await;
    drop(provider);
    drop(spawner);
    drop(worker);
}

pub(super) async fn pending_reap_failure_preserves_launch_cause_and_original_retry_owner() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = DrainWorker::new(false, true);
    let spawner = Spawner::new(worker.clone(), true);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let provisioning = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(spec("held-pending", root)).await }
    });
    spawner.entered.notified().await;
    let error = provider.drain_generation(&scope).await.unwrap_err();
    assert!(error.to_string().contains("generation drain requested"));
    assert!(
        error
            .to_string()
            .contains("original child remains uncertain")
    );
    let launch_error = provisioning.await.unwrap().err().unwrap();
    assert!(
        launch_error
            .to_string()
            .contains("generation drain requested")
    );
    assert!(
        launch_error
            .to_string()
            .contains("original child remains uncertain")
    );
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("held-pending").is_dir());
    assert!(cgroups.join("held-pending").is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&VmId("held-pending".into()))
    );
    assert!(LibkrunProvider::open_owned(config.clone(), &scope).is_err());
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    provider.drain_generation(&scope).await.unwrap();
    provider.drain_generation(&scope).await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 2);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 2);
    assert!(worker.worker.process_exit.borrow().is_some());
    assert!(!runtime.join("held-pending").exists());
    assert!(!cgroups.join("held-pending").exists());
    assert!(
        provider
            .ownership_test_inner()
            .pending_launches
            .lock()
            .unwrap()
            .is_empty()
    );
    drop(provider);
    drop(spawner);
    drop(worker);
}

async fn assert_cancelled_drain_completed(
    provider: &LibkrunProvider,
    scope: &vm_trait::VmProviderOwnerScope,
    worker: &DrainWorker,
    runtime: &std::path::Path,
    cgroups: &std::path::Path,
) {
    let inventory = provider.attempt_inventory(scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::Cleaned);
    assert_eq!(inventory.attempts[0].operations, 0);
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    assert!(!runtime.join("pending").exists());
    assert!(!cgroups.join("pending").exists());
    assert!(provider.ownership_test_inner().ids.lock().await.is_empty());
}

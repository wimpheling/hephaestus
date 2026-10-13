//! Existing unchanged scenario bodies; parent retains original test names.
use super::super::support::*;
use super::{DrainWorker, Spawner, owned};
use crate::provider::VmAttemptPhase;
use std::{future::Future, task::Context};

pub(super) async fn handed_off_reap_failure_retains_exact_instance_for_drain_retry() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = DrainWorker::new(false, true);
    let spawner = Spawner::new(worker.clone(), false);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("instance", root)).await.unwrap();
    let error = provider.drain_generation(&scope).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("original child remains uncertain")
    );
    assert!(runtime.join("instance").is_dir());
    assert!(cgroups.join("instance").is_dir());
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(vm.start().await.is_err());
    assert_eq!(worker.worker.start_calls.load(Ordering::Relaxed), 0);
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(vm.id())
    );
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::CleanupPending);
    assert!(
        inventory.attempts[0]
            .error
            .as_ref()
            .unwrap()
            .contains("original child remains uncertain")
    );
    provider.drain_generation(&scope).await.unwrap();
    vm.destroy().await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 2);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 2);
    assert_eq!(spawner.calls.load(Ordering::Relaxed), 1);
    assert!(!runtime.join("instance").exists());
    assert!(!cgroups.join("instance").exists());
    drop(vm);
    drop(provider);
    drop(spawner);
    drop(worker);
}

pub(super) async fn concurrent_drains_clean_one_actual_instance_once_and_preserve_history() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = DrainWorker::new(false, false);
    let spawner = Spawner::new(worker.clone(), false);
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("concurrent", root)).await.unwrap();
    vm.start().await.unwrap();
    let (first, second) = tokio::join!(
        provider.drain_generation(&scope),
        provider.drain_generation(&scope)
    );
    first.unwrap();
    second.unwrap();
    provider.drain_generation(&scope).await.unwrap();
    vm.destroy().await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts.len(), 1);
    assert_eq!(inventory.attempts[0].birth, 1);
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::Cleaned);
    assert_eq!(inventory.attempts[0].operations, 0);
    assert!(provider.ownership_test_inner().ids.lock().await.is_empty());
    assert!(!runtime.join("concurrent").exists());
    assert!(!cgroups.join("concurrent").exists());
    drop(vm);
    drop(provider);
    drop(spawner);
    drop(worker);
}

pub(super) async fn drain_waits_for_owning_lifecycle_task_queued_before_operation_mutex() {
    let temp = TempDir::new().unwrap();
    let (config, root, _, _) = emulated_config(&temp);
    let worker = DrainWorker::new(false, false);
    let provider = owned(&config, Spawner::new(worker.clone(), false));
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("queued", root)).await.unwrap();
    let instance = provider
        .ownership_test_inner()
        .registry
        .instances()
        .pop()
        .unwrap();
    vm.destroy().await.unwrap();
    provider
        .ownership_test_inner()
        .registry
        .wait_settled()
        .await;
    let operation = instance.operation.lock().await;
    let mut queued = Box::pin(vm.start());
    // Poll the public future once: registration/spawn are synchronous, and its
    // owning task must now queue behind the actual instance's operation mutex.
    assert!(
        queued
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    let mut draining = Box::pin(provider.drain_generation(&scope));
    // This negative deadline drops only the borrowed waiter, preserving both
    // owning tasks. Without queued-task bookkeeping drain would return early.
    assert!(
        tokio::time::timeout(Duration::from_millis(1), &mut draining)
            .await
            .is_err()
    );
    drop(operation);
    assert!(matches!(
        queued.await,
        Err(VmError::InvalidState("VM provider admission is closed"))
    ));
    draining.await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    drop(instance);
    drop(vm);
    drop(provider);
    drop(worker);
}

pub(super) async fn drain_deadline_reports_uncertainty_without_canceling_held_instance_cleanup() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    config.startup_timeout = Duration::from_millis(10);
    let worker = DrainWorker::new(true, false);
    let provider = owned(&config, Spawner::new(worker.clone(), false));
    let scope = provider.owner_scope().unwrap();
    let vm = provider.provision(spec("deadline", root)).await.unwrap();
    let draining = tokio::spawn({
        let provider = provider.clone();
        let scope = scope.clone();
        async move { provider.drain_generation(&scope).await }
    });
    worker.kill_entered.notified().await;
    tokio::time::advance(Duration::from_millis(10)).await;
    let error = draining.await.unwrap().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("instance cleanup remains unconfirmed")
    );
    assert!(worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join("deadline").is_dir());
    assert!(cgroups.join("deadline").is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(vm.id())
    );
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts[0].phase, VmAttemptPhase::CleanupPending);
    assert_eq!(inventory.attempts[0].operations, 1);
    worker.release_kill.notify_one();
    provider.drain_generation(&scope).await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.reaps.load(Ordering::Relaxed), 1);
    assert!(!runtime.join("deadline").exists());
    assert!(!cgroups.join("deadline").exists());
    drop(vm);
    drop(provider);
    drop(worker);
}

//! Actual provider token/launch custody with controlled backend + emulated dirs.
//! These tests are NOT native VM/cgroup or prior-process cleanup proof.
#[path = "attempt/support.rs"]
mod controls;
use super::support::*;
use controls::{FailedKill, HeldKill, Spawner, bounded, raw};

#[tokio::test]
async fn preparation_and_unused_cleanup_have_no_allocation_or_orphan_fallback() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let provider = LibkrunProvider::new_with_spawner(config, Arc::new(FailingSpawner)).unwrap();
    let token = provider
        .prepare_provision_attempt(spec("unused", root))
        .unwrap();
    assert!(fs::read_dir(&runtime).unwrap().next().is_none());
    assert!(fs::read_dir(&cgroups).unwrap().next().is_none());
    // Same name is unrelated to this unused token; its cleanup may not look it up.
    fs::create_dir(runtime.join("unused")).unwrap();
    fs::write(runtime.join("unused/sentinel"), b"unrelated").unwrap();
    bounded(token.cleanup()).await.unwrap();
    bounded(token.cleanup()).await.unwrap();
    assert!(matches!(
        bounded(token.provision()).await,
        Err(VmError::Destroyed)
    ));
    assert_eq!(
        fs::read(runtime.join("unused/sentinel")).unwrap(),
        b"unrelated"
    );
    drop(provider);
}

#[tokio::test]
async fn concurrent_provision_returns_same_original_and_cleans_original_mount_backing_only() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let spawner = Spawner::new(vec![worker.clone()], true, false);
    let provider = LibkrunProvider::new_with_spawner(config, spawner.clone()).unwrap();
    let mount = temp.path().join("mounts/source");
    fs::create_dir(&mount).unwrap();
    fs::write(mount.join("source"), b"caller backing").unwrap();
    let mut requested = spec("concurrent", root);
    requested.mounts.push(vm_trait::VmMount {
        tag: "source".into(),
        host_path: mount.clone(),
        guest_path: "/workspace/source".into(),
        read_only: true,
    });
    let token = provider.prepare_provision_attempt(requested).unwrap();
    let first = tokio::spawn({
        let token = token.clone();
        async move { token.provision().await }
    });
    bounded(spawner.entered.notified()).await;
    let second = tokio::spawn({
        let token = token.clone();
        async move { token.provision().await }
    });
    spawner.release.notify_one();
    let first = bounded(first).await.unwrap().unwrap();
    let second = bounded(second).await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
    bounded(token.cleanup()).await.unwrap();
    assert!(worker.process_exit.borrow().is_some());
    assert!(!runtime.join("concurrent").exists());
    assert!(!cgroups.join("concurrent").exists());
    assert_eq!(fs::read(mount.join("source")).unwrap(), b"caller backing");
    drop(second);
    drop(first);
    drop(token);
    drop(provider);
}

#[tokio::test]
async fn abandoned_provision_waiter_cannot_cancel_original_owner_and_cleanup_joins_pending_birth() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let spawner = Spawner::new(vec![worker.clone()], true, false);
    let provider = LibkrunProvider::new_with_spawner(config, spawner.clone()).unwrap();
    let token = provider
        .prepare_provision_attempt(spec("abandoned", root))
        .unwrap();
    let waiter = tokio::spawn({
        let token = token.clone();
        async move { token.provision().await }
    });
    bounded(spawner.entered.notified()).await;
    waiter.abort();
    assert!(bounded(waiter).await.err().unwrap().is_cancelled());
    assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
    // Never release spawn: exact captured drain must settle that pending owner.
    bounded(token.cleanup()).await.unwrap();
    assert!(worker.process_exit.borrow().is_some());
    assert!(!runtime.join("abandoned").exists());
    assert!(!cgroups.join("abandoned").exists());
    drop(provider);
}

#[tokio::test]
async fn failed_held_launch_retains_same_raw_error_until_explicit_same_token_cleanup_retry() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let backend = Arc::new(FailedKill {
        worker: worker.clone(),
        failures: AtomicUsize::new(2),
    });
    let spawner = Spawner::new(vec![backend], false, true);
    let provider = LibkrunProvider::new_with_spawner(config, spawner.clone()).unwrap();
    let token = provider
        .prepare_provision_attempt(spec("held", root))
        .unwrap();
    let first = bounded(token.provision()).await.err().unwrap();
    let again = bounded(token.provision()).await.err().unwrap();
    assert!(std::ptr::eq(raw(&first), raw(&again)));
    assert!(worker.process_exit.borrow().is_none());
    assert!(runtime.join("held").exists());
    assert!(cgroups.join("held").exists());
    assert!(bounded(token.cleanup()).await.is_err());
    assert!(worker.process_exit.borrow().is_none());
    assert!(runtime.join("held").exists());
    bounded(token.cleanup()).await.unwrap();
    assert!(worker.process_exit.borrow().is_some());
    assert!(!runtime.join("held").exists());
    assert!(!cgroups.join("held").exists());
    assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        bounded(token.provision()).await,
        Err(VmError::Destroyed)
    ));
    drop(provider);
}

#[tokio::test]
async fn rejected_same_id_token_never_cleans_other_original_instance() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, _) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let spawner = Spawner::new(vec![worker.clone()], false, false);
    let provider = LibkrunProvider::new_with_spawner(config, spawner).unwrap();
    let first = provider
        .prepare_provision_attempt(spec("same-id", root.clone()))
        .unwrap();
    let instance = bounded(first.provision()).await.unwrap();
    let rejected = provider
        .prepare_provision_attempt(spec("same-id", root))
        .unwrap();
    let error = bounded(rejected.provision()).await.err().unwrap();
    assert!(matches!(raw(&error), VmError::AlreadyExists(_)));
    bounded(rejected.cleanup()).await.unwrap();
    assert!(worker.process_exit.borrow().is_none());
    assert!(runtime.join("same-id").exists());
    bounded(first.cleanup()).await.unwrap();
    drop(instance);
    drop(provider);
}

#[tokio::test]
async fn cleaned_old_token_never_touches_successor_reusing_id() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, _) = emulated_config(&temp);
    let old = Arc::new(MockWorker::new());
    let next = Arc::new(MockWorker::new());
    let spawner = Spawner::new(vec![old.clone(), next.clone()], false, false);
    let provider = LibkrunProvider::new_with_spawner(config, spawner).unwrap();
    let first = provider
        .prepare_provision_attempt(spec("reused", root.clone()))
        .unwrap();
    let original = bounded(first.provision()).await.unwrap();
    bounded(first.cleanup()).await.unwrap();
    let second = provider
        .prepare_provision_attempt(spec("reused", root))
        .unwrap();
    let successor = bounded(second.provision()).await.unwrap();
    bounded(first.cleanup()).await.unwrap();
    assert!(next.process_exit.borrow().is_none());
    assert!(runtime.join("reused").exists());
    assert!(matches!(
        bounded(first.provision()).await,
        Err(VmError::Destroyed)
    ));
    bounded(second.cleanup()).await.unwrap();
    drop(successor);
    drop(original);
    drop(provider);
}

#[tokio::test]
async fn abandoned_cleanup_reply_keeps_same_original_destroy_and_blocks_later_provision() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let held = Arc::new(HeldKill {
        worker: worker.clone(),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let provider =
        LibkrunProvider::new_with_spawner(config, Spawner::new(vec![held.clone()], false, false))
            .unwrap();
    let token = provider
        .prepare_provision_attempt(spec("cleanup-waiter", root))
        .unwrap();
    let instance = bounded(token.provision()).await.unwrap();
    let waiter = tokio::spawn({
        let token = token.clone();
        async move { token.cleanup().await }
    });
    bounded(held.entered.notified()).await;
    waiter.abort();
    assert!(bounded(waiter).await.err().unwrap().is_cancelled());
    assert!(worker.process_exit.borrow().is_none());
    assert!(runtime.join("cleanup-waiter").exists());
    held.release.notify_one();
    bounded(token.cleanup()).await.unwrap();
    assert!(worker.process_exit.borrow().is_some());
    assert!(!runtime.join("cleanup-waiter").exists());
    assert!(!cgroups.join("cleanup-waiter").exists());
    assert!(matches!(
        bounded(token.provision()).await,
        Err(VmError::Destroyed)
    ));
    drop(instance);
    drop(provider);
}

#[path = "attempt/handoff.rs"]
mod handoff;

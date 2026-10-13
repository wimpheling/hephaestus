//! Actual pre-ack launch owner controls, NOT native process/cgroup evidence.
use super::*;
use crate::provider::{launch::LaunchState, provider_api::prepare_close_on_capture};

#[tokio::test]
async fn captured_pre_ack_instance_cannot_clean_token_while_original_owner_is_blocked() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    config.startup_timeout = Duration::from_millis(100);
    let worker = Arc::new(MockWorker::new());
    let held = Arc::new(HeldKill {
        worker: worker.clone(),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let provider =
        LibkrunProvider::new_with_spawner(config, Spawner::new(vec![held.clone()], false, false))
            .unwrap();
    // Test-only close executes synchronously after SAME instance capture and
    // before accepted.send. Native owner handles the genuinely failed ack.
    let token = prepare_close_on_capture(provider.clone(), spec("pre-ack-blocked", root));
    assert!(matches!(
        raw(&bounded(token.provision()).await.err().unwrap()),
        VmError::Destroyed
    ));
    bounded(held.entered.notified()).await;
    let inner = provider.launch_test_inner();
    let pending = inner
        .pending_launches
        .lock()
        .unwrap()
        .get(&VmId("pre-ack-blocked".into()))
        .cloned()
        .unwrap();
    assert!(matches!(*pending.state.borrow(), LaunchState::Handoff));
    let error = bounded(token.cleanup()).await.err().unwrap();
    assert!(matches!(raw(&error), VmError::Unavailable { .. }));
    assert!(matches!(*pending.state.borrow(), LaunchState::Handoff));
    assert!(worker.process_exit.borrow().is_none());
    assert!(runtime.join("pre-ack-blocked").exists());
    assert!(cgroups.join("pre-ack-blocked").exists());
    // SAME retained owner continues after the failed reply. Retry can succeed
    // only after its real cleanup and owning launch task have both settled.
    held.release.notify_one();
    bounded(token.cleanup()).await.unwrap();
    assert!(matches!(*pending.state.borrow(), LaunchState::Cleaned));
    assert!(inner.pending_launches.lock().unwrap().is_empty());
    assert!(worker.process_exit.borrow().is_some());
    assert!(!runtime.join("pre-ack-blocked").exists());
    assert!(!cgroups.join("pre-ack-blocked").exists());
    drop(pending);
    drop(inner);
    drop(provider);
}

#[tokio::test]
async fn captured_pre_ack_held_cleanup_retries_same_owner_before_success_and_preserves_successor() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let old = Arc::new(MockWorker::new());
    let next = Arc::new(MockWorker::new());
    let backend = Arc::new(FailedKill {
        worker: old.clone(),
        failures: AtomicUsize::new(1),
    });
    let spawner = Spawner::new(vec![backend, next.clone()], false, false);
    let provider = LibkrunProvider::new_with_spawner(config, spawner.clone()).unwrap();
    let token = prepare_close_on_capture(provider.clone(), spec("pre-ack-held", root.clone()));
    assert!(matches!(
        raw(&bounded(token.provision()).await.err().unwrap()),
        VmError::Destroyed
    ));
    let inner = provider.launch_test_inner();
    let pending = inner
        .pending_launches
        .lock()
        .unwrap()
        .get(&VmId("pre-ack-held".into()))
        .cloned()
        .unwrap();
    let mut state = pending.state.subscribe();
    bounded(state.wait_for(|state| matches!(state, LaunchState::Held(_))))
        .await
        .unwrap();
    drop(state);
    assert!(old.process_exit.borrow().is_none());
    assert!(runtime.join("pre-ack-held").exists());
    assert!(cgroups.join("pre-ack-held").exists());
    bounded(token.cleanup()).await.unwrap();
    // Direct same-instance destroy alone could succeed while this exact native
    // launch owner still waits in Held. Positive token cleanup requires Cleaned.
    assert!(matches!(*pending.state.borrow(), LaunchState::Cleaned));
    assert!(inner.pending_launches.lock().unwrap().is_empty());
    assert!(old.process_exit.borrow().is_some());
    drop(pending);
    drop(inner);
    let successor = provider
        .prepare_provision_attempt(spec("pre-ack-held", root))
        .unwrap();
    let instance = bounded(successor.provision()).await.unwrap();
    bounded(token.cleanup()).await.unwrap();
    assert!(next.process_exit.borrow().is_none());
    assert!(runtime.join("pre-ack-held").exists());
    assert_eq!(spawner.calls.load(Ordering::SeqCst), 2);
    bounded(successor.cleanup()).await.unwrap();
    drop(instance);
    drop(provider);
}

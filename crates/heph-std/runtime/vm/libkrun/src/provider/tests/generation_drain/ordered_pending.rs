//! Existing unchanged scenario bodies; parent retains original test names.
use super::super::support::*;
use super::{DrainWorker, Spawner, TwoSpawners, assert_pending_failure, owned};
use crate::provider::launch::{LaunchState, PendingLaunch};

pub(super) async fn later_pending_failure_cannot_be_retried_after_waiting_on_earlier_birth() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let earlier_worker = DrainWorker::new(true, false);
    let later_worker = DrainWorker::new(false, true);
    let earlier = Spawner::new(earlier_worker.clone(), true);
    let later = Spawner::new(later_worker.clone(), true);
    let provider = owned(
        &config,
        Arc::new(TwoSpawners(std::sync::Mutex::new(
            std::collections::VecDeque::from([earlier.clone(), later.clone()]),
        ))),
    );
    let scope = provider.owner_scope().unwrap();
    let first_provision = tokio::spawn({
        let provider = provider.clone();
        let requested = spec("earlier", root.clone());
        async move { provider.provision(requested).await }
    });
    earlier.entered.notified().await;
    let second_provision = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(spec("later", root)).await }
    });
    later.entered.notified().await;
    let later_entry = provider
        .ownership_test_inner()
        .pending_launches
        .lock()
        .unwrap()
        .get(&VmId("later".into()))
        .cloned()
        .unwrap();
    let mut later_state = later_entry.state.subscribe();
    let draining = tokio::spawn({
        let provider = provider.clone();
        let scope = scope.clone();
        async move { provider.drain_generation(&scope).await }
    });
    earlier_worker.kill_entered.notified().await;
    // Observe the real later owner's first failure while the earlier kill still
    // blocks the sequential drain pass. No delay or fabricated Held state.
    assert_pending_failure(&mut later_state).await;
    assert!(!draining.is_finished());
    assert_eq!(later_worker.reaps.load(Ordering::Relaxed), 1);
    earlier_worker.release_kill.notify_one();
    let error = draining.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("generation drain requested"));
    assert!(
        error
            .to_string()
            .contains("original child remains uncertain")
    );
    assert!(first_provision.await.unwrap().is_err());
    let refused = second_provision.await.unwrap().err().unwrap();
    assert!(
        refused
            .to_string()
            .contains("original child remains uncertain")
    );
    assert_later_held(&provider, &later_entry, &later_worker, &runtime, &cgroups).await;
    provider.drain_generation(&scope).await.unwrap();
    assert_retry_completed(
        &provider,
        &earlier_worker,
        &later_worker,
        &earlier,
        &later,
        &runtime,
        &cgroups,
    )
    .await;
    drop(later_entry);
    drop(provider);
    drop(earlier);
    drop(later);
    drop(earlier_worker);
    drop(later_worker);
}

async fn assert_later_held(
    provider: &LibkrunProvider,
    later_entry: &Arc<PendingLaunch>,
    later_worker: &DrainWorker,
    runtime: &std::path::Path,
    cgroups: &std::path::Path,
) {
    assert!(matches!(*later_entry.state.borrow(), LaunchState::Held(_)));
    assert_eq!(later_worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(later_worker.reaps.load(Ordering::Relaxed), 1);
    assert!(later_worker.worker.process_exit.borrow().is_none());
    for allocation in [&runtime, &cgroups] {
        assert!(!allocation.join("earlier").exists());
        assert!(allocation.join("later").is_dir());
    }
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&VmId("later".into()))
    );
    assert!(Arc::ptr_eq(
        later_entry,
        provider
            .ownership_test_inner()
            .pending_launches
            .lock()
            .unwrap()
            .get(&VmId("later".into()))
            .unwrap()
    ));
}

async fn assert_retry_completed(
    provider: &LibkrunProvider,
    earlier_worker: &DrainWorker,
    later_worker: &DrainWorker,
    earlier: &Spawner,
    later: &Spawner,
    runtime: &std::path::Path,
    cgroups: &std::path::Path,
) {
    assert_eq!(later_worker.kills.load(Ordering::Relaxed), 2);
    assert_eq!(later_worker.reaps.load(Ordering::Relaxed), 2);
    assert!(later_worker.worker.process_exit.borrow().is_some());
    assert_eq!(earlier_worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(earlier_worker.reaps.load(Ordering::Relaxed), 1);
    assert_eq!(earlier.calls.load(Ordering::Relaxed), 1);
    assert_eq!(later.calls.load(Ordering::Relaxed), 1);
    assert!(provider.ownership_test_inner().ids.lock().await.is_empty());
    assert!(!runtime.join("later").exists());
    assert!(!cgroups.join("later").exists());
}

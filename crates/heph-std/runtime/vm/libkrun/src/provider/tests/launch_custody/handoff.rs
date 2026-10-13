//! Existing unchanged scenario bodies; parent retains original test names.
use super::super::support::*;
use super::{held, spawner};
use crate::provider::{
    launch::{LaunchState, PendingLaunch, run_launch},
    ownership::ProviderOwner,
};

pub(super) async fn cancelled_provision_retains_original_owner_resources_and_id_until_retry() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let spawner = spawner(false);
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let requested = spec("cancelled-launch", root);
    let id = requested.id.clone();
    let task = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(requested).await }
    });
    spawner.entered.notified().await;
    // Abort only the public caller, never the private launch owner or worker.
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let entry = provider
        .ownership_test_inner()
        .pending_launches
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .unwrap();
    held(&entry).await;
    assert!(spawner.worker.worker.process_exit.borrow().is_none());
    assert!(runtime.join(&id.0).is_dir());
    assert!(cgroups.join(&id.0).is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(matches!(
        provider
            .provision(spec(&id.0, temp.path().join("images/root")))
            .await,
        Err(VmError::AlreadyExists(_))
    ));
    drop(provider);
    assert!(
        ProviderOwner::initialize(&config, "test-host").is_err(),
        "uncertain launch must retain original supervisor"
    );
    entry.retry_cleanup().await.unwrap();
    entry.retry_cleanup().await.unwrap();
    assert!(spawner.worker.worker.process_exit.borrow().is_some());
    assert_eq!(spawner.worker.reaps.load(Ordering::Relaxed), 2);
    assert!(!runtime.join(&id.0).exists());
    assert!(!cgroups.join(&id.0).exists());
    let reopened = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(owner) = ProviderOwner::initialize(&config, "test-host") {
                break owner;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(reopened);
    drop(entry);
    drop(spawner);
}

pub(super) async fn configure_failure_preserves_cause_and_uncertainty_for_exact_scoped_retry() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let spawner = spawner(true);
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let scope = owner.scope().unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let requested = spec("failed-configure", root);
    let id = requested.id.clone();
    let task = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(requested).await }
    });
    spawner.entered.notified().await;
    spawner.release.notify_one();
    let error = task.await.unwrap().err().unwrap();
    assert!(error.to_string().contains("controlled configure failure"));
    assert!(error.to_string().contains("first wait failed"));
    let entry = provider
        .ownership_test_inner()
        .pending_launches
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .unwrap();
    let state = entry.state.borrow().clone();
    let LaunchState::Held(failure) = state else {
        panic!("launch not retained");
    };
    assert!(
        failure
            .cause
            .to_string()
            .contains("controlled configure failure")
    );
    assert!(failure.cleanup.to_string().contains("first wait failed"));
    assert!(runtime.join(&id.0).is_dir());
    assert!(cgroups.join(&id.0).is_dir());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    provider.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    assert!(
        provider
            .ownership_test_inner()
            .pending_launches
            .lock()
            .unwrap()
            .get(&id)
            .is_none()
    );
    assert!(
        !provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(!runtime.join(&id.0).exists());
    assert!(!cgroups.join(&id.0).exists());
    drop(failure);
    drop(entry);
    drop(provider);
    drop(spawner);
}

pub(super) async fn lost_queued_success_requires_instance_cleanup_before_entry_retirement() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let spawner = spawner(false);
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let requested = spec("lost-handoff", root);
    let prepared = prepare_spec(&config, &requested).unwrap();
    let (entry, retries) = PendingLaunch::new(requested.id.clone(), &config, None).unwrap();
    let inner = provider.launch_test_inner();
    {
        let mut ids = inner.ids.lock().await;
        inner
            .registry
            .admit(&inner.lifetime, &mut ids, entry.clone())
            .unwrap();
    }
    inner
        .pending_launches
        .lock()
        .unwrap()
        .insert(entry.id.clone(), entry.clone());
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(run_launch(
        inner.clone(),
        entry.clone(),
        prepared,
        None,
        sender,
        retries,
    ));
    spawner.entered.notified().await;
    let mut state = entry.state.subscribe();
    spawner.release.notify_one();
    state
        .wait_for(|state| matches!(state, LaunchState::Handoff))
        .await
        .unwrap();
    // The result is queued, but no actual receiver acknowledgement occurred.
    drop(receiver);
    held(&entry).await;
    assert!(inner.ids.lock().await.contains(&entry.id));
    assert!(runtime.join(&entry.id.0).is_dir());
    assert!(cgroups.join(&entry.id.0).is_dir());
    assert!(spawner.worker.worker.process_exit.borrow().is_none());
    provider.cleanup_orphan(&entry.id).await.unwrap();
    task.await.unwrap();
    assert!(!inner.ids.lock().await.contains(&entry.id));
    assert!(inner.pending_launches.lock().unwrap().is_empty());
    assert!(!runtime.join(&entry.id.0).exists());
    drop(state);
    drop(inner);
    drop(entry);
    drop(provider);
    drop(spawner);
}

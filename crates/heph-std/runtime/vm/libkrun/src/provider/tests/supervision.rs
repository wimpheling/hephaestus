use super::{ownership_provision::PausedSpawner, support::*};
use crate::provider::ownership::ProviderOwner;

fn owned(config: &LibkrunConfig, spawner: Arc<dyn WorkerSpawner>) -> LibkrunProvider {
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner).unwrap();
    assert!(
        provider
            .ownership_test_inner()
            .owner
            .set(ProviderOwner::initialize(config, "test-host").unwrap())
            .is_ok()
    );
    provider
}

#[tokio::test]
async fn clones_retain_exclusive_supervisor_until_last_provider_drops() {
    let temp = TempDir::new().unwrap();
    let (config, _root, _runtime, _cgroups) = emulated_config(&temp);
    let provider = owned(&config, Arc::new(FailingSpawner));
    let expected = provider.owner_scope().unwrap();
    let cloned = provider.clone();
    drop(provider);
    assert!(LibkrunProvider::new_owned(config.clone(), "test-host").is_err());
    assert_eq!(cloned.owner_scope().unwrap(), expected);
    drop(cloned);
    let restarted = LibkrunProvider::new_owned(config, "test-host").unwrap();
    assert_eq!(restarted.owner_scope().unwrap(), expected);
    drop(restarted);
}

#[tokio::test]
async fn unowned_constructor_and_preexisting_unowned_handle_cannot_bypass_marked_root() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, _cgroups) = emulated_config(&temp);
    let unowned =
        LibkrunProvider::new_with_spawner(config.clone(), Arc::new(FailingSpawner)).unwrap();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    assert!(LibkrunProvider::new(config).is_err());
    assert!(unowned.provision(spec("bypass", root)).await.is_err());
    assert!(
        unowned
            .cleanup_orphan(&VmId("bypass".into()))
            .await
            .is_err()
    );
    drop(unowned);
    assert!(!runtime.join("bypass").exists());
    drop(owner);
}

#[tokio::test]
async fn two_vms_reach_paused_spawners_concurrently_with_one_supervisor() {
    let temp = TempDir::new().unwrap();
    let (config, root, _runtime, _cgroups) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let spawner = Arc::new(PausedSpawner {
        worker,
        entered: Notify::new(),
        release: Notify::new(),
    });
    let provider = owned(&config, spawner.clone());
    let scope = provider.owner_scope().unwrap();
    let first = tokio::spawn({
        let provider = provider.clone();
        let root = root.clone();
        async move { provider.provision(spec("parallel-a", root)).await }
    });
    spawner.entered.notified().await;
    let second = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(spec("parallel-b", root)).await }
    });
    tokio::time::timeout(Duration::from_secs(1), spawner.entered.notified())
        .await
        .unwrap();
    assert_eq!(provider.owner_scope().unwrap(), scope);
    assert!(LibkrunProvider::new_owned(config.clone(), "test-host").is_err());
    spawner.release.notify_waiters();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(first.id(), &VmId("parallel-a".into()));
    assert_eq!(second.id(), &VmId("parallel-b".into()));
    drop(provider);
    // Live instances and their monitor tasks retain the supervisor, even after
    // the facade disappears. They cannot be adopted by an independent provider.
    assert!(LibkrunProvider::new_owned(config, "test-host").is_err());
    first.destroy().await.unwrap();
    second.destroy().await.unwrap();
}

use super::support::*;
use crate::provider::ownership::ProviderOwner;

#[tokio::test]
async fn scoped_cleanup_checks_owner_and_live_workers_before_absence() {
    let temp = TempDir::new().unwrap();
    let (config, _root, runtime, cgroups) = emulated_config(&temp);
    let provider =
        LibkrunProvider::new_with_spawner(config.clone(), Arc::new(FailingSpawner)).unwrap();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let scope = owner.scope().unwrap();
    // This unit fixture injects validated ownership without a native VM.
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let id = VmId("orphan".into());
    let wrong =
        vm_trait::VmProviderOwnerScope::new("another-owner".into(), "test-host".into()).unwrap();
    assert!(provider.cleanup_orphan_scoped(&wrong, &id).await.is_err());
    provider.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    fs::create_dir(runtime.join("orphan")).unwrap();
    fs::write(runtime.join("orphan").join("owned"), "owned runtime").unwrap();
    fs::create_dir(cgroups.join("orphan")).unwrap();
    provider
        .ownership_test_inner()
        .ids
        .lock()
        .await
        .insert(id.clone());
    assert!(provider.cleanup_orphan_scoped(&scope, &id).await.is_err());
    assert!(runtime.join("orphan").is_dir());
    provider.ownership_test_inner().ids.lock().await.remove(&id);
    provider.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    assert!(!runtime.join("orphan").exists());
    assert!(!cgroups.join("orphan").exists());
    fs::rename(&runtime, temp.path().join("old-runtime")).unwrap();
    fs::create_dir(&runtime).unwrap();
    assert!(
        provider.cleanup_orphan_scoped(&scope, &id).await.is_err(),
        "missing target on replacement root is not authoritative absence"
    );
    drop(provider);
}

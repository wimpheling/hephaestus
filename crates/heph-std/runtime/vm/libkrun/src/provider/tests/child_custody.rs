//! Actual admitted births with controlled backend/filesystem substitutions.
use super::support::*;
use crate::provider::ownership::ProviderOwner;

struct Spawner(Arc<MockWorker>);
#[async_trait]
impl WorkerSpawner for Spawner {
    async fn spawn(
        &self,
        _config: Arc<LibkrunConfig>,
        _spec: crate::validation::PreparedSpec,
        _runtime: &std::path::Path,
        _cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        let worker: Arc<dyn WorkerBackend> = self.0.clone();
        custody.record_backend(Arc::clone(&worker))?;
        Ok(worker)
    }
}

#[tokio::test]
async fn replaced_runtime_child_keeps_birth_id_and_original_resources_until_restored_retry() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, _) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let provider =
        LibkrunProvider::new_with_spawner(config.clone(), Arc::new(Spawner(worker.clone())))
            .unwrap();
    let owner = ProviderOwner::initialize(&config, "child-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let scope = provider.owner_scope().unwrap();
    let requested = spec("vm", root);
    let vm = provider.provision(requested.clone()).await.unwrap();
    let id = VmId("vm".into());
    let original = temp.path().join("original-runtime-child");
    fs::write(runtime.join("vm/owned"), b"original").unwrap();
    fs::rename(runtime.join("vm"), &original).unwrap();
    fs::create_dir(runtime.join("vm")).unwrap();
    fs::write(runtime.join("vm/sentinel"), b"foreign").unwrap();
    assert!(vm.destroy().await.is_err());
    assert!(worker.process_exit.borrow().is_some());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(provider.provision(requested).await.is_err());
    assert_eq!(fs::read(runtime.join("vm/sentinel")).unwrap(), b"foreign");
    assert_eq!(fs::read(original.join("owned")).unwrap(), b"original");
    let inventory = provider.attempt_inventory(&scope).await.unwrap();
    assert_eq!(inventory.attempts.len(), 1);
    assert!(inventory.attempts[0].error.is_some());
    fs::rename(
        runtime.join("vm"),
        temp.path().join("foreign-runtime-child"),
    )
    .unwrap();
    fs::rename(original, runtime.join("vm")).unwrap();
    vm.destroy().await.unwrap();
    assert!(
        !provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(!runtime.join("vm").exists());
    assert_eq!(
        fs::read(temp.path().join("foreign-runtime-child/sentinel")).unwrap(),
        b"foreign"
    );
    drop(vm);
    drop(worker);
    drop(provider);
}

#[tokio::test]
async fn replaced_cgroup_child_cannot_receive_controls_or_release_original_birth() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let provider =
        LibkrunProvider::new_with_spawner(config.clone(), Arc::new(Spawner(worker.clone())))
            .unwrap();
    let owner = ProviderOwner::initialize(&config, "child-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let vm = provider.provision(spec("vm", root)).await.unwrap();
    let original = temp.path().join("original-cgroup-child");
    fs::rename(cgroups.join("vm"), &original).unwrap();
    fs::create_dir(cgroups.join("vm")).unwrap();
    fs::write(cgroups.join("vm/cgroup.procs"), b"foreign").unwrap();
    assert!(vm.destroy().await.is_err());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&VmId("vm".into()))
    );
    assert!(runtime.join("vm").is_dir());
    assert!(original.is_dir());
    assert_eq!(
        fs::read(cgroups.join("vm/cgroup.procs")).unwrap(),
        b"foreign"
    );
    fs::rename(cgroups.join("vm"), temp.path().join("foreign-cgroup-child")).unwrap();
    fs::rename(original, cgroups.join("vm")).unwrap();
    vm.destroy().await.unwrap();
    assert!(!runtime.join("vm").exists());
    assert!(!cgroups.join("vm").exists());
    assert_eq!(
        fs::read(temp.path().join("foreign-cgroup-child/cgroup.procs")).unwrap(),
        b"foreign"
    );
    drop(vm);
    drop(worker);
    drop(provider);
}

#[tokio::test]
async fn replacement_during_pending_launch_refuses_handoff_and_retains_exact_retry() {
    use super::ownership_provision::PausedSpawner;
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, _) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let spawner = Arc::new(PausedSpawner {
        worker: worker.clone(),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let owner = ProviderOwner::initialize(&config, "child-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let scope = provider.owner_scope().unwrap();
    let admitted = provider.clone();
    let provision = tokio::spawn(async move { admitted.provision(spec("vm", root)).await });
    spawner.entered.notified().await;
    let original = temp.path().join("original-runtime-child");
    fs::write(runtime.join("vm/owned"), b"original").unwrap();
    fs::rename(runtime.join("vm"), &original).unwrap();
    fs::create_dir(runtime.join("vm")).unwrap();
    fs::write(runtime.join("vm/sentinel"), b"foreign").unwrap();
    spawner.release.notify_one();
    let error = provision.await.unwrap().err().unwrap().to_string();
    assert!(error.contains("runtime-identity"));
    assert!(error.contains("runtime-cleanup"));
    let id = VmId("vm".into());
    assert!(worker.process_exit.borrow().is_some());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert_eq!(fs::read(runtime.join("vm/sentinel")).unwrap(), b"foreign");
    assert_eq!(fs::read(original.join("owned")).unwrap(), b"original");
    fs::rename(
        runtime.join("vm"),
        temp.path().join("foreign-runtime-child"),
    )
    .unwrap();
    fs::rename(original, runtime.join("vm")).unwrap();
    provider.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    assert!(
        !provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(!runtime.join("vm").exists());
    assert_eq!(
        fs::read(temp.path().join("foreign-runtime-child/sentinel")).unwrap(),
        b"foreign"
    );
    drop(worker);
    drop(spawner);
    drop(provider);
}

use super::support::*;
use crate::provider::ownership::ProviderOwner;

pub(super) struct PausedSpawner {
    pub(super) worker: Arc<dyn WorkerBackend>,
    pub(super) entered: Notify,
    pub(super) release: Notify,
}

#[async_trait]
impl WorkerSpawner for PausedSpawner {
    async fn spawn(
        &self,
        _config: Arc<LibkrunConfig>,
        _spec: crate::validation::PreparedSpec,
        _runtime: &std::path::Path,
        _cgroup: &Cgroup,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(self.worker.clone())
    }
}

#[tokio::test]
async fn replacement_during_spawn_destroys_exact_worker_and_original_resources() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let spawner = Arc::new(PausedSpawner {
        worker: worker.clone(),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let requested = spec("swapped-root-vm", root);
    let provisioning = provider.clone();
    let task = tokio::spawn(async move { provisioning.provision(requested).await });
    spawner.entered.notified().await;
    // Independent ownership lookup succeeds while another VM owns a shared IO
    // guard; it must not depend on that VM's allocation finishing first.
    provider.owner_scope().unwrap();
    let old = temp.path().join("old-runtime");
    fs::rename(&runtime, &old).unwrap();
    fs::create_dir(&runtime).unwrap();
    fs::create_dir(runtime.join("swapped-root-vm")).unwrap();
    let sentinel = runtime.join("swapped-root-vm").join("unclassified");
    fs::write(&sentinel, b"replacement bytes").unwrap();
    spawner.release.notify_one();
    assert!(
        task.await.unwrap().is_err(),
        "ownership changed during spawn cannot return a VM"
    );
    assert!(
        !provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&VmId("swapped-root-vm".into()))
    );
    drop(provider);
    assert!(
        worker.process_exit.borrow().is_some(),
        "exact worker must be reaped before returning"
    );
    assert!(!old.join("swapped-root-vm").exists());
    assert!(!cgroups.join("swapped-root-vm").exists());
    assert_eq!(fs::read(sentinel).unwrap(), b"replacement bytes");
}

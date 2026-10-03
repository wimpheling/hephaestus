use super::{ownership_provision::PausedSpawner, support::*};
use crate::provider::ownership::ProviderOwner;
use std::sync::atomic::AtomicBool;

struct FailedFirstReap {
    worker: Arc<MockWorker>,
    fail: AtomicBool,
}

#[async_trait]
impl WorkerBackend for FailedFirstReap {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        if matches!(command, WorkerCommand::Destroy) {
            return Err(unavailable_error(
                "mock worker destroy",
                "termination uncertain",
            ));
        }
        self.worker.request(command).await
    }
    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
        self.worker.subscribe_events()
    }
    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        self.worker.subscribe_process_exit()
    }
    async fn kill(&self) -> Result<(), VmError> {
        self.worker.kill().await
    }
    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        if self.fail.swap(false, Ordering::Relaxed) {
            return Err(unavailable_error(
                "mock worker reap",
                "termination unconfirmed",
            ));
        }
        self.worker.wait_process().await
    }
}

#[tokio::test]
async fn failed_destroy_retains_worker_and_blocks_id_reuse_until_scoped_recovery() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = Arc::new(MockWorker::new());
    let backend = Arc::new(FailedFirstReap {
        worker: worker.clone(),
        fail: AtomicBool::new(true),
    });
    let spawner = Arc::new(PausedSpawner {
        worker: backend,
        entered: Notify::new(),
        release: Notify::new(),
    });
    let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let scope = owner.scope().unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let requested = spec("uncertain-vm", root);
    let retry = requested.clone();
    let provisioning = provider.clone();
    let task = tokio::spawn(async move { provisioning.provision(requested).await });
    spawner.entered.notified().await;
    let old = temp.path().join("old-runtime");
    fs::rename(&runtime, &old).unwrap();
    fs::create_dir(&runtime).unwrap();
    spawner.release.notify_one();
    assert!(task.await.unwrap().is_err());
    let id = VmId("uncertain-vm".into());
    assert!(
        worker.process_exit.borrow().is_none(),
        "failed reap is not physical destruction"
    );
    assert!(
        provider
            .ownership_test_inner()
            .failed_cleanup
            .lock()
            .await
            .contains_key(&id)
    );
    assert!(old.join(&id.0).exists());
    assert!(cgroups.join(&id.0).exists());
    assert!(
        provider.cleanup_orphan_scoped(&scope, &id).await.is_err(),
        "redirected current root cannot confirm cleanup"
    );
    assert!(
        fs::read_dir(&runtime).unwrap().next().is_none(),
        "denied validation must not create owner metadata in the replacement root"
    );
    fs::remove_dir(&runtime).unwrap();
    fs::rename(&old, &runtime).unwrap();
    assert!(matches!(
        provider.provision(retry).await,
        Err(VmError::AlreadyExists(_))
    ));
    provider.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    assert!(
        !provider
            .ownership_test_inner()
            .failed_cleanup
            .lock()
            .await
            .contains_key(&id)
    );
    assert!(
        !provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    drop(provider);
    assert!(worker.process_exit.borrow().is_some());
    assert!(!runtime.join(&id.0).exists());
    assert!(!cgroups.join(&id.0).exists());
}

//! Controlled current-generation negative; no native absence claim.
use super::support::*;
use crate::provider::ownership::ProviderOwner;

struct Spawner(Arc<MockWorker>);
#[async_trait]
impl WorkerSpawner for Spawner {
    async fn spawn(
        &self,
        _: Arc<LibkrunConfig>,
        _: crate::validation::PreparedSpec,
        _: &std::path::Path,
        _: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        let worker: Arc<dyn WorkerBackend> = self.0.clone();
        custody.record_backend(worker.clone())?;
        Ok(worker)
    }
}

#[tokio::test]
async fn actual_controlled_birth_blocks_settled_observer_until_destroy_retirement() {
    let temp = TempDir::new().unwrap();
    let (config, root, _, _) = emulated_config(&temp);
    let provider = LibkrunProvider::new_with_spawner(
        config.clone(),
        Arc::new(Spawner(Arc::new(MockWorker::new()))),
    )
    .unwrap();
    let scope = vm_trait::VmProviderOwnerScope::new(
        uuid::Uuid::new_v4().to_string(),
        "controlled-observer-host".into(),
    )
    .unwrap();
    let owner = ProviderOwner::bootstrap(&config, &scope).unwrap();
    assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
    let instance = provider
        .provision(spec("observer-live-birth", root))
        .await
        .unwrap();
    {
        let ids = provider.ownership_test_inner().ids.lock().await;
        assert!(
            provider
                .ownership_test_inner()
                .registry
                .observe_settled::<()>(|| panic!("active birth must reject before observation"))
                .is_err()
        );
        drop(ids);
    }
    assert!(
        !provider
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .admission_closed
    );
    // Actual existing current-generation cleanup, not prior-process emulation.
    instance.destroy().await.unwrap();
    provider.drain_generation(&scope).await.unwrap();
    {
        let ids = provider.ownership_test_inner().ids.lock().await;
        assert!(ids.is_empty());
        provider
            .ownership_test_inner()
            .registry
            .observe_settled(|| Ok(()))
            .unwrap();
        drop(ids);
    }
    // Complete public observation still refuses this uncontrolled fake filesystem.
    assert!(provider.assert_no_prior_births(&scope).await.is_err());
    drop(provider);
}

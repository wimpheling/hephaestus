use run_domain::{LegacyVmPlacementConsumption, LegacyVmPlacementScope};
use run_orchestrator::RunRepository;
use vm_trait::VmProviderOwnerScope;

use super::support::{MemoryRepository, normal_stateless_command};

#[tokio::test]
async fn existing_repositories_do_not_imply_placement_closure_or_inventory_support() {
    let input = normal_stateless_command();
    let repository = MemoryRepository::new(&input);
    let scope = LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(
            runtime_types::RunId::new().to_string(),
            "contract-host".into(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        repository
            .create_run_with_legacy_placement(&input, &scope)
            .await
            .is_err()
    );
    assert!(
        repository
            .consume_legacy_vm_placement(&input, &scope, LegacyVmPlacementConsumption::Provision)
            .await
            .is_err()
    );
    assert!(
        repository
            .legacy_vm_placement(input.run_id, &scope)
            .await
            .is_err()
    );
    assert!(
        repository
            .close_legacy_vm_acquisition(input.run_id, &scope)
            .await
            .is_err()
    );
    assert!(
        repository
            .legacy_vm_placement_inventory(&scope)
            .await
            .is_err()
    );
    assert!(!*repository.created.lock().await);
    assert!(repository.events.lock().await.is_empty());
    assert!(repository.run.lock().await.vm_id.is_none());
}

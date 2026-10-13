use crate::FakeProvider;
use crate::tests::spec;
use vm_trait::{VmError, VmId, VmProvider};

#[tokio::test]
async fn fresh_fake_cannot_confirm_another_owner_absent() {
    let first = FakeProvider::new_owned("test-host".into()).unwrap();
    let scope = first.owner_scope().unwrap();
    assert_eq!(first.clone().owner_scope().unwrap(), scope);
    let replacement = FakeProvider::new_owned("test-host".into()).unwrap();
    assert_ne!(replacement.owner_scope().unwrap(), scope);
    let id = VmId("owned-vm".into());
    assert!(
        replacement
            .cleanup_orphan_scoped(&scope, &id)
            .await
            .is_err()
    );
    first.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    let vm = first.provision(spec(&id.0)).await.unwrap();
    assert!(first.cleanup_orphan_scoped(&scope, &id).await.is_err());
    vm.destroy().await.unwrap();
    first.cleanup_orphan_scoped(&scope, &id).await.unwrap();
}

#[tokio::test]
async fn unowned_fake_never_falls_back_to_unscoped_success() {
    let provider = FakeProvider::new();
    let scope = vm_trait::VmProviderOwnerScope::new("owner".into(), "host".into()).unwrap();
    let id = VmId("absent".into());
    provider.cleanup_orphan(&id).await.unwrap();
    assert!(matches!(
        provider.owner_scope(),
        Err(VmError::Unsupported { .. })
    ));
    assert!(matches!(
        provider.cleanup_orphan_scoped(&scope, &id).await,
        Err(VmError::Unsupported { .. })
    ));
}

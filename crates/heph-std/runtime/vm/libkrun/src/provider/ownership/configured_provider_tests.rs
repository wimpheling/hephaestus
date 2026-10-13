use super::*;
use crate::provider::LibkrunProvider;
use std::fs;
use tempfile::TempDir;
use vm_trait::VmProvider;

#[test]
fn public_configured_constructors_keep_clone_supervisor_and_exact_scope() {
    if !super::configured_tests::isolated_positive(
        "provider::ownership::configured_provider_tests::public_configured_constructors_keep_clone_supervisor_and_exact_scope",
    ) {
        return;
    }
    let temp = TempDir::new().unwrap();
    let mut config = super::tests::config(&temp);
    config.kvm_device = temp.path().join("emulated-kvm");
    config.passt_binary = "/bin/true".into();
    fs::write(&config.kvm_device, b"").unwrap();
    let scope =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "configured-host".into())
            .unwrap();
    let provider = LibkrunProvider::bootstrap_owned(config.clone(), &scope).unwrap();
    assert_eq!(provider.owner_scope().unwrap(), scope);
    let cloned = provider.clone();
    drop(provider);
    assert!(LibkrunProvider::open_owned(config.clone(), &scope).is_err());
    assert!(LibkrunProvider::bootstrap_owned(config.clone(), &scope).is_err());
    assert_eq!(cloned.owner_scope().unwrap(), scope);
    drop(cloned);
    let reopened = LibkrunProvider::open_owned(config, &scope).unwrap();
    assert_eq!(reopened.owner_scope().unwrap(), scope);
    drop(reopened);
}

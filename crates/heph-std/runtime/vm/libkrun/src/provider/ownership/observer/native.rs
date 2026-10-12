//! Genuine native positive control; no worker/guest is spawned by this test.
use crate::{LibkrunConfig, LibkrunProvider};
use std::{fs, path::PathBuf};
use tempfile::TempDir;
use vm_trait::{VmProvider, VmProviderOwnerScope};

#[tokio::test]
#[ignore = "requires qualified image/worker assets and an empty dedicated real cgroup-v2 delegation"]
async fn actual_delegation_readonly_observation_and_same_scope_reopen() {
    let temp = TempDir::new().unwrap();
    let runtime = temp.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    let image = PathBuf::from(
        std::env::var_os("HEPHAESTUS_LIBKRUN_OWNED_ROOTFS").expect("qualified image root"),
    );
    let cgroup = PathBuf::from(
        std::env::var_os("HEPHAESTUS_LIBKRUN_OWNED_CGROUP_ROOT")
            .expect("task-owned empty real delegation"),
    );
    let worker = PathBuf::from(
        std::env::var_os("HEPHAESTUS_LIBKRUN_CONTAINMENT_WORKER")
            .expect("actual compiled worker binary"),
    );
    let config = LibkrunConfig::new(
        runtime.clone(),
        vec![image.clone()],
        vec![image.clone()],
        vec![image],
        worker,
        cgroup.clone(),
    );
    let scope = VmProviderOwnerScope::new(
        uuid::Uuid::new_v4().to_string(),
        "native-observer-host".into(),
    )
    .unwrap();
    let provider = LibkrunProvider::bootstrap_owned(config.clone(), &scope).unwrap();
    let original_marker = fs::read(runtime.join(".heph-vm-owner.json")).unwrap();
    let before = provider.attempt_inventory(&scope).await.unwrap();
    assert!(before.attempts.is_empty());
    assert!(!before.admission_closed);
    provider.assert_no_prior_births(&scope).await.unwrap();
    assert_eq!(provider.attempt_inventory(&scope).await.unwrap(), before);
    assert_eq!(
        fs::read(runtime.join(".heph-vm-owner.json")).unwrap(),
        original_marker
    );

    let residual = runtime.join(".heph-vm-birth-unrecognized.json");
    fs::write(&residual, b"partial test-owned record").unwrap();
    assert!(provider.assert_no_prior_births(&scope).await.is_err());
    assert_eq!(fs::read(&residual).unwrap(), b"partial test-owned record");
    fs::remove_file(residual).unwrap(); // Remove only our negative fixture input.
    let child = cgroup.join("observer-test-residual");
    fs::create_dir(&child).unwrap();
    assert!(provider.assert_no_prior_births(&scope).await.is_err());
    assert!(child.is_dir());
    fs::remove_dir(child).unwrap(); // No processes or VM were placed here.
    provider.assert_no_prior_births(&scope).await.unwrap();
    assert!(
        !provider
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .admission_closed
    );
    drop(provider);
    let reopened = LibkrunProvider::open_owned(config, &scope).unwrap();
    assert_eq!(reopened.owner_scope().unwrap(), scope);
    reopened.assert_no_prior_births(&scope).await.unwrap();
    assert_eq!(
        fs::read(runtime.join(".heph-vm-owner.json")).unwrap(),
        original_marker
    );
    drop(reopened);
}

//! Actual KVM exercise of configured bootstrap and existing-only reopen.
use super::fixture;
use std::{fs, time::Duration};
use vm_libkrun::LibkrunProvider;
use vm_trait::{VmProvider, VmProviderOwnerScope};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires fresh guest root, delegated cgroup and actual KVM"]
async fn configured_owner_bootstrap_reopen_and_actual_scoped_cleanup() {
    let fixture = fixture::Fixture::new();
    let scope =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), fixture::HOST.into()).unwrap();
    assert!(LibkrunProvider::open_owned(fixture.config.clone(), &scope).is_err());
    assert_eq!(
        fs::read_dir(&fixture.config.runtime_root).unwrap().count(),
        0
    );
    let provider = LibkrunProvider::bootstrap_owned(fixture.config.clone(), &scope).unwrap();
    fixture.assert_marker(&scope);
    assert_eq!(provider.owner_scope().unwrap(), scope);
    assert!(LibkrunProvider::open_owned(fixture.config.clone(), &scope).is_err());
    let vm = provider
        .provision(fixture.spec("configured-native-live"))
        .await
        .unwrap();
    let id = vm.id().clone();
    let pids =
        fs::read_to_string(fixture.config.cgroup_root.join(&id.0).join("cgroup.procs")).unwrap();
    vm.start().await.unwrap();
    fixture.assert_live(&id);
    let clone = provider.clone();
    drop(provider);
    assert!(LibkrunProvider::open_owned(fixture.config.clone(), &scope).is_err());
    vm.destroy().await.unwrap();
    fixture.assert_clean(&id);
    for pid in pids.split_whitespace() {
        assert!(!std::path::Path::new("/proc").join(pid).exists());
    }
    let weak = std::sync::Arc::downgrade(&vm);
    drop(vm);
    tokio::time::timeout(Duration::from_secs(2), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(LibkrunProvider::open_owned(fixture.config.clone(), &scope).is_err());
    drop(clone);
    let before = fixture::entries(&fixture.config.runtime_root);
    let reopened = LibkrunProvider::open_owned(fixture.config.clone(), &scope).unwrap();
    assert_eq!(reopened.owner_scope().unwrap(), scope);
    assert_eq!(fixture::entries(&fixture.config.runtime_root), before);
    reopened.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    let foreign =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), fixture::HOST.into()).unwrap();
    assert!(reopened.cleanup_orphan_scoped(&foreign, &id).await.is_err());
    assert_eq!(fixture::entries(&fixture.config.runtime_root), before);
    drop(reopened);
    println!(
        "REAL_CONFIGURED_VM_OWNER_KVM=1 protocol=11 configured_namespace=1 bootstrap=1 readonly_reopen=1 provision_start_destroy=1 scoped_absence=1 worker_pid_gone=1 clone_supervisor=1 no_app=1 namespace={}",
        scope.namespace()
    );
}

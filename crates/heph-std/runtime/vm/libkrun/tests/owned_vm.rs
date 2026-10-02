//! Opt-in real KVM proof of persistent provider ownership and scoped cleanup.

#[path = "owned_vm/fixture.rs"]
mod fixture;
#[path = "owned_vm/process.rs"]
mod process;

use std::{fs, time::Duration};
use vm_libkrun::LibkrunProvider;
use vm_trait::{VmProvider, VmProviderOwnerScope};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires fresh guest root, delegated cgroup and actual KVM"]
async fn real_owned_vm_lifetime_scoped_cleanup_and_supervisor_restart() {
    let fixture = fixture::Fixture::new();
    let provider = LibkrunProvider::new_owned(fixture.config.clone(), fixture::HOST)
        .expect("fresh actual owned provider");
    let scope = provider.owner_scope().unwrap();
    fixture.assert_marker(&scope);
    process::reject_second_supervisor(&fixture).await;
    let clone = provider.clone();
    drop(provider);
    assert!(LibkrunProvider::new_owned(fixture.config.clone(), fixture::HOST).is_err());
    let vm = clone
        .provision(fixture.spec("owned-native-live"))
        .await
        .unwrap();
    vm.start().await.expect("real protocol10 KVM readiness");
    fixture.assert_live(vm.id());
    assert!(clone.cleanup_orphan_scoped(&scope, vm.id()).await.is_err());
    drop(clone);
    assert!(LibkrunProvider::new_owned(fixture.config.clone(), fixture::HOST).is_err());
    let id = vm.id().clone();
    let weak = std::sync::Arc::downgrade(&vm);
    vm.destroy().await.expect("actual worker killed and reaped");
    fixture.assert_clean(&id);
    drop(vm);
    tokio::time::timeout(Duration::from_secs(2), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("completed monitor tasks release the exact destroyed VM");
    let reopened = LibkrunProvider::new_owned(fixture.config.clone(), fixture::HOST).unwrap();
    assert_eq!(reopened.owner_scope().unwrap(), scope);
    reopened.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    wrong_scope_and_replaced_root(&fixture, &reopened, &scope, &id).await;
    drop(reopened);
    process::native_supervisor_death_and_restart(&fixture, &scope).await;
    println!(
        "REAL_OWNED_VM_KVM=1 protocol=10 actual_provision_start_destroy=1 real_cgroup_worker=1 instance_clone_supervisor_retained=1 separate_supervisor_denied=1 scoped_absence=1 wrong_scope_replaced_root_untouched=1 native_parent_death_restart_cleanup=1 namespace={}",
        scope.namespace()
    );
}

async fn wrong_scope_and_replaced_root(
    fixture: &fixture::Fixture,
    provider: &LibkrunProvider,
    scope: &VmProviderOwnerScope,
    id: &vm_trait::VmId,
) {
    let before = fixture::entries(&fixture.config.runtime_root);
    let foreign = VmProviderOwnerScope::new(scope.namespace().into(), "wrong-host".into()).unwrap();
    assert!(provider.cleanup_orphan_scoped(&foreign, id).await.is_err());
    assert_eq!(fixture::entries(&fixture.config.runtime_root), before);
    let original = fixture.directory.path().join("original-runtime");
    fs::rename(&fixture.config.runtime_root, &original).unwrap();
    fs::create_dir(&fixture.config.runtime_root).unwrap();
    fs::write(fixture.config.runtime_root.join("foreign"), b"untouched").unwrap();
    let before = fixture::entries(&fixture.config.runtime_root);
    assert!(provider.cleanup_orphan_scoped(scope, id).await.is_err());
    assert!(provider.owner_scope().is_err());
    assert_eq!(fixture::entries(&fixture.config.runtime_root), before);
    fs::remove_file(fixture.config.runtime_root.join("foreign")).unwrap();
    fs::remove_dir(&fixture.config.runtime_root).unwrap();
    fs::rename(original, &fixture.config.runtime_root).unwrap();
    assert_eq!(provider.owner_scope().unwrap(), *scope);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "subprocess helper; only invoked with task-owned fixture environment"]
async fn owned_vm_child_session() {
    let Some(mode) = std::env::var_os("HEPH_OWNED_VM_CHILD_MODE") else {
        return;
    };
    let fixture = fixture::Fixture::child();
    if mode == "probe" {
        assert!(LibkrunProvider::new_owned(fixture.config.clone(), fixture::HOST).is_err());
        println!("OWNED_VM_SEPARATE_SUPERVISOR_DENIED=1");
        return;
    }
    assert_eq!(mode, "vm");
    let provider = LibkrunProvider::new_owned(fixture.config.clone(), fixture::HOST).unwrap();
    let scope = provider.owner_scope().unwrap();
    let vm = provider
        .provision(fixture.spec("owned-native-orphan"))
        .await
        .unwrap();
    drop(provider);
    vm.start().await.unwrap();
    fixture.assert_live(vm.id());
    let ready = std::env::var_os("HEPH_OWNED_VM_CHILD_READY").unwrap();
    fs::write(ready, scope.namespace()).unwrap();
    drop(fixture);
    // The parent kills this supervisor while the real guest remains running.
    tokio::time::sleep(Duration::from_secs(60)).await;
    panic!("parent failed to terminate native supervisor fixture");
}

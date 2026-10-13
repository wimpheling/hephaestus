use super::fixture::{Fixture, HOST};
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};
use vm_libkrun::LibkrunProvider;
use vm_trait::{VmId, VmProvider, VmProviderOwnerScope};

struct ChildSession(Child);
impl Drop for ChildSession {
    fn drop(&mut self) {
        let _kill = self.0.kill();
        let _wait = self.0.wait();
    }
}

fn command(fixture: &Fixture, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "owned_vm_child_session",
            "--ignored",
            "--nocapture",
        ])
        .env("HEPH_OWNED_VM_CHILD_MODE", mode)
        .env("HEPH_OWNED_VM_CHILD_RUNTIME", &fixture.config.runtime_root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
}

pub async fn reject_second_supervisor(fixture: &Fixture) {
    let mut child = ChildSession(command(fixture, "probe").spawn().unwrap());
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bounded supervisor rejection");
}

pub async fn native_supervisor_death_and_restart(fixture: &Fixture, scope: &VmProviderOwnerScope) {
    let ready = fixture.directory.path().join("child-ready");
    let mut command = command(fixture, "vm");
    command.env("HEPH_OWNED_VM_CHILD_READY", &ready);
    let mut child = ChildSession(command.spawn().unwrap());
    tokio::time::timeout(Duration::from_secs(45), async {
        while !ready.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "native child exited before readiness"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("real child VM readiness");
    assert_eq!(std::fs::read_to_string(&ready).unwrap(), scope.namespace());
    assert!(LibkrunProvider::new_owned(fixture.config.clone(), HOST).is_err());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    drop(child);
    let provider = LibkrunProvider::new_owned(fixture.config.clone(), HOST).unwrap();
    assert_eq!(provider.owner_scope().unwrap(), *scope);
    let id = VmId("owned-native-orphan".into());
    assert!(
        fixture.config.runtime_root.join(&id.0).exists(),
        "actual persisted orphan resources"
    );
    provider
        .cleanup_orphan_scoped(scope, &id)
        .await
        .expect("authoritative cleanup after actual supervisor death");
    fixture.assert_clean(&id);
    provider.cleanup_orphan_scoped(scope, &id).await.unwrap();
    drop(provider);
    println!(
        "OWNED_VM_REAL_SUPERVISOR_RESTART=1 same_namespace=1 actual_orphan_cgroup_destroyed=1 scoped_absence=1"
    );
}

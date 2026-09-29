// Scenario tests intentionally retain Arc and broker handles across awaits so
// concurrent lifecycle behavior remains observable by each task.
#![allow(clippy::significant_drop_tightening)]
use super::support::*;

#[tokio::test]
async fn failed_worker_launch_cleans_runtime_and_cgroup() {
    let temp = TempDir::new().unwrap();
    let runtime = temp.path().join("runtime");
    let images = temp.path().join("images");
    let disks = temp.path().join("disks");
    let mounts = temp.path().join("mounts");
    let cgroups = temp.path().join("cgroups");
    for directory in [&runtime, &images, &disks, &mounts, &cgroups] {
        fs::create_dir(directory).unwrap();
    }
    let root = images.join("root");
    fs::create_dir(&root).unwrap();
    let caller_disk = disks.join("agent-state.raw");
    fs::write(&caller_disk, b"caller-owned-state").unwrap();
    let kvm = temp.path().join("kvm");
    fs::write(&kvm, "").unwrap();
    let mut config = LibkrunConfig::new(
        &runtime,
        vec![images],
        vec![disks],
        vec![mounts],
        "/bin/false",
        &cgroups,
    );
    config.passt_binary = PathBuf::from("/bin/false");
    config.kvm_device = kvm;
    config.enforce_cgroup_v2 = false;
    config.startup_timeout = Duration::from_millis(50);
    let provider = LibkrunProvider::new(config).unwrap();

    let mut requested = spec("worker-failure", root);
    requested.disks.push(VmDisk {
        id: String::from("instance-state"),
        host_path: caller_disk.clone(),
        format: DiskFormat::Raw,
        read_only: false,
    });
    let error = provider
        .provision(requested)
        .await
        .err()
        .expect("provision must fail");
    assert!(matches!(error, VmError::Unavailable { .. }));
    assert!(!runtime.join("worker-failure").exists());
    assert!(!cgroups.join("worker-failure").exists());
    assert_eq!(
        fs::read(&caller_disk).unwrap(),
        b"caller-owned-state",
        "failed provisioning modified caller-owned disk"
    );
}
#[tokio::test]
async fn orphan_cleanup_preserves_caller_owned_backing() {
    let temp = TempDir::new().unwrap();
    let (config, _root, runtime, cgroups) = emulated_config(&temp);
    let disk = temp.path().join("disks").join("agent-state.raw");
    fs::write(&disk, b"persistent").unwrap();
    fs::create_dir(runtime.join("orphan")).unwrap();
    fs::write(runtime.join("orphan").join("socket"), []).unwrap();
    fs::create_dir(cgroups.join("orphan")).unwrap();
    let provider = LibkrunProvider::new_with_spawner(config, Arc::new(FailingSpawner)).unwrap();

    provider
        .cleanup_orphan(&VmId(String::from("orphan")))
        .await
        .unwrap();

    assert!(!runtime.join("orphan").exists());
    assert!(!cgroups.join("orphan").exists());
    assert_eq!(fs::read(disk).unwrap(), b"persistent");
}
#[tokio::test]
async fn injected_spawner_failure_cleans_every_allocated_resource() {
    let temp = TempDir::new().unwrap();
    let (config, root, runtime, cgroups) = emulated_config(&temp);
    let provider = LibkrunProvider::new_with_spawner(config, Arc::new(FailingSpawner)).unwrap();
    let error = provider
        .provision(spec("spawner-failure", root))
        .await
        .err()
        .expect("injected worker spawn must fail");
    assert!(matches!(
        error,
        VmError::Unavailable { resource, .. } if resource == "worker spawn"
    ));
    assert!(!runtime.join("spawner-failure").exists());
    assert!(!cgroups.join("spawner-failure").exists());
}

//! Opt-in hardware integration tests for the Fedora libkrun backend.

use std::{
    collections::BTreeMap,
    env,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    DiskFormat, GuestCommand, NetworkMode, PortForward, PortProtocol, RootFilesystem, StopMode,
    VmDisk, VmEvent, VmId, VmMount, VmProvider, VmResources, VmSpec,
};

const ENABLE_FLAG: &str = "HEPHAESTUS_LIBKRUN_INTEGRATION";

#[tokio::test(flavor = "multi_thread")]
async fn boots_and_exercises_guest_runtime_without_privilege_escalation() {
    if env::var(ENABLE_FLAG).as_deref() != Ok("1") {
        return;
    }

    let runtime_root = required_path("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT");
    let image_root = required_path("HEPHAESTUS_LIBKRUN_IMAGE_ROOT");
    let rootfs = required_path("HEPHAESTUS_LIBKRUN_ROOTFS");
    let disk_root = required_path("HEPHAESTUS_LIBKRUN_DISK_ROOT");
    let sqlite_disk = required_path("HEPHAESTUS_LIBKRUN_SQLITE_DISK");
    let mount_root = required_path("HEPHAESTUS_LIBKRUN_MOUNT_ROOT");
    let repository = required_path("HEPHAESTUS_LIBKRUN_REPOSITORY");
    let workspace = required_path("HEPHAESTUS_LIBKRUN_WORKSPACE");
    let cgroup_root = required_path("HEPHAESTUS_LIBKRUN_CGROUP_ROOT");
    let cgroup_root_for_assertion = cgroup_root.clone();
    let rootfs_for_force_test = rootfs.clone();

    let mut config = LibkrunConfig::new(
        &runtime_root,
        vec![image_root],
        vec![disk_root],
        vec![mount_root],
        env!("CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"),
        cgroup_root,
    );
    config.startup_timeout = Duration::from_secs(15);
    config.readiness_timeout = Duration::from_secs(45);
    let provider = LibkrunProvider::new(config).expect("integration host configuration");
    let spec = integration_spec(rootfs, sqlite_disk, repository, workspace);

    let vm = provider.provision(spec).await.expect("provision VM");
    let vm_id = vm.id().0.clone();
    let mut events = vm.subscribe_events();
    let first = Arc::clone(&vm);
    let second = Arc::clone(&vm);
    let (first, second) = tokio::join!(first.start(), second.start());
    first.expect("first concurrent start");
    second.expect("second concurrent start");

    let mut markers = String::new();
    let exit = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            match events.recv().await.expect("ordered VM event") {
                VmEvent::Started { ingress } => {
                    assert_ne!(ingress[0].host_port, 0);
                }
                VmEvent::Log { bytes, .. } => {
                    markers.push_str(&String::from_utf8_lossy(&bytes));
                }
                VmEvent::Exited(exit) => break exit,
                _ => {}
            }
        }
    })
    .await
    .expect("guest completion timeout");

    assert_eq!(exit.code, Some(0));
    for marker in ["sqlite=ok", "dns=ok", "tcp=ok", "udp=ok"] {
        assert!(markers.contains(marker), "missing guest marker {marker}");
    }
    assert_eq!(vm.wait().await.expect("cached exit"), exit);
    vm.stop(StopMode::Graceful {
        timeout: Duration::from_secs(2),
    })
    .await
    .expect("idempotent graceful stop");
    vm.destroy().await.expect("complete cleanup");
    vm.destroy().await.expect("idempotent cleanup");
    assert!(!runtime_root.join(&vm_id).exists());
    assert!(!cgroup_root_for_assertion.join(&vm_id).exists());

    let forced = provider
        .provision(force_cleanup_spec(rootfs_for_force_test))
        .await
        .expect("provision forced-cleanup VM");
    let forced_id = forced.id().0.clone();
    forced.start().await.expect("start forced-cleanup VM");
    forced.destroy().await.expect("force cleanup running VM");
    let forced_exit = forced.wait().await.expect("forced exit is cached");
    assert!(forced_exit.signal.is_some() || forced_exit.code.is_some());
    assert!(!runtime_root.join(&forced_id).exists());
    assert!(!cgroup_root_for_assertion.join(&forced_id).exists());
}

fn integration_spec(
    rootfs: PathBuf,
    sqlite_disk: PathBuf,
    repository: PathBuf,
    workspace: PathBuf,
) -> VmSpec {
    VmSpec {
        id: VmId(format!("integration-{}", std::process::id())),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: vec![VmDisk {
            id: "sqlite".to_owned(),
            host_path: sqlite_disk,
            format: DiskFormat::Raw,
            read_only: false,
        }],
        mounts: vec![
            VmMount {
                tag: "repository".to_owned(),
                host_path: repository,
                guest_path: PathBuf::from("/repository"),
                read_only: true,
            },
            VmMount {
                tag: "workspace".to_owned(),
                host_path: workspace,
                guest_path: PathBuf::from("/workspace"),
                read_only: false,
            },
        ],
        resources: VmResources {
            vcpus: 2,
            memory_mib: 1024,
        },
        network: NetworkMode::UserMode {
            ingress: vec![PortForward {
                protocol: PortProtocol::Tcp,
                bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                host_port: 0,
                guest_port: 8080,
            }],
        },
        command: GuestCommand {
            program: "/usr/libexec/hephaestus/integration-check".to_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/workspace")),
        },
        labels: BTreeMap::from([("test".to_owned(), "hardware".to_owned())]),
    }
}

fn required_path(name: &str) -> PathBuf {
    env::var_os(name).map_or_else(
        || panic!("{name} must be set when {ENABLE_FLAG}=1"),
        PathBuf::from,
    )
}

fn force_cleanup_spec(rootfs: PathBuf) -> VmSpec {
    VmSpec {
        id: VmId(format!("integration-force-{}", std::process::id())),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: "/bin/sleep".to_owned(),
            args: vec!["300".to_owned()],
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/")),
        },
        labels: BTreeMap::from([("test".to_owned(), "forced-cleanup".to_owned())]),
    }
}

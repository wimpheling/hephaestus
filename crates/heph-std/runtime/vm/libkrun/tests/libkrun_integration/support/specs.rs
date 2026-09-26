use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use vm_conformance::ProviderHarness;
use vm_libkrun::LibkrunProvider;
use vm_trait::{
    DiskFormat, GuestCommand, NetworkMode, PortForward, PrivateHttpServiceSpec, RootFilesystem,
    VmDisk, VmId, VmMount, VmResources, VmSpec,
};
use vm_trait::{PortProtocol, VmProvider};

pub fn state_probe_spec(
    id: &str,
    rootfs: PathBuf,
    sqlite_disk: PathBuf,
    sqlite_uuid: &str,
    argument: &str,
) -> VmSpec {
    VmSpec {
        id: VmId(id.to_owned()),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: vec![VmDisk {
            id: "instance-state".to_owned(),
            host_path: sqlite_disk,
            format: DiskFormat::Raw,
            read_only: false,
        }],
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: String::from("/usr/libexec/hephaestus/integration-check"),
            args: vec![argument.to_owned()],
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/")),
        },
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::from([
            ("test".to_owned(), id.to_owned()),
            (
                "hephaestus.agent-state.filesystem-uuid".to_owned(),
                sqlite_uuid.to_owned(),
            ),
            (
                "hephaestus.agent-state.mount-path".to_owned(),
                "/var/lib/hephaestus".to_owned(),
            ),
        ]),
    }
}

pub struct LibkrunHarness {
    pub provider: Arc<LibkrunProvider>,
    pub rootfs: PathBuf,
    pub runtime_root: PathBuf,
    pub cgroup_root: PathBuf,
}

impl ProviderHarness for LibkrunHarness {
    fn provider(&self) -> Arc<dyn VmProvider> {
        self.provider.clone()
    }

    fn long_running_spec(&self, id: &str) -> VmSpec {
        VmSpec {
            id: VmId(id.to_owned()),
            root: RootFilesystem::Directory {
                host_path: self.rootfs.clone(),
            },
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
            runtime_authority: None,
            private_http_service: None,
            runtime_git_bridge: None,
            labels: BTreeMap::from([("test".to_owned(), "conformance".to_owned())]),
        }
    }

    fn ephemeral_ingress_spec(&self, id: &str) -> Option<VmSpec> {
        let mut spec = self.long_running_spec(id);
        spec.network = NetworkMode::UserMode {
            ingress: vec![PortForward {
                protocol: PortProtocol::Tcp,
                bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                host_port: 0,
                guest_port: 22,
            }],
        };
        Some(spec)
    }

    fn assert_clean(&self, id: &VmId) {
        assert!(!self.runtime_root.join(&id.0).exists());
        assert!(!self.cgroup_root.join(&id.0).exists());
    }
}

pub fn integration_spec(
    id: &str,
    rootfs: PathBuf,
    sqlite_disk: PathBuf,
    sqlite_uuid: &str,
    repository: PathBuf,
    workspace: PathBuf,
    secret_mount: Option<VmMount>,
) -> VmSpec {
    let mut mounts = vec![
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
    ];
    let expects_secrets = secret_mount.is_some();
    mounts.extend(secret_mount);
    VmSpec {
        id: VmId(id.to_owned()),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: vec![VmDisk {
            id: "instance-state".to_owned(),
            host_path: sqlite_disk,
            format: DiskFormat::Raw,
            read_only: false,
        }],
        mounts,
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
            env: if expects_secrets {
                BTreeMap::from([(String::from("HEPH_EXPECT_SECRET_MOUNT"), String::from("1"))])
            } else {
                BTreeMap::new()
            },
            working_dir: Some(PathBuf::from("/workspace")),
        },
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::from([
            ("test".to_owned(), "hardware".to_owned()),
            (
                "hephaestus.agent-state.filesystem-uuid".to_owned(),
                sqlite_uuid.to_owned(),
            ),
            (
                "hephaestus.agent-state.mount-path".to_owned(),
                "/var/lib/hephaestus".to_owned(),
            ),
        ]),
    }
}

pub fn mode_spec(rootfs: PathBuf, id: &str, argument: &str, network: NetworkMode) -> VmSpec {
    VmSpec {
        id: VmId(id.to_owned()),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        network,
        command: GuestCommand {
            program: String::from("/usr/libexec/hephaestus/integration-check"),
            args: vec![argument.to_owned()],
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/")),
        },
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::from([("test".to_owned(), id.to_owned())]),
    }
}

pub fn private_http_spec(rootfs: PathBuf) -> VmSpec {
    VmSpec {
        id: VmId(format!("integration-private-http-{}", std::process::id())),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: "/usr/libexec/hephaestus/integration-check".to_owned(),
            args: vec!["--private-http-handler".to_owned()],
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/")),
        },
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::from([(
            "hephaestus.gateway.handler-contract".to_owned(),
            "http.v1".to_owned(),
        )]),
    }
}

pub fn private_service_spec(rootfs: PathBuf, id: String) -> VmSpec {
    VmSpec {
        id: VmId(id),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        network: NetworkMode::Disabled,
        private_http_service: Some(PrivateHttpServiceSpec {
            loopback_port: 8080,
            max_connections: 2,
            connect_timeout: Duration::from_secs(2),
        }),
        command: GuestCommand {
            program: "/usr/libexec/hephaestus/integration-check".to_owned(),
            args: vec!["--serve-service".to_owned()],
            env: BTreeMap::from([
                (
                    String::from("HEPH_SERVICE_STARTUP_DELAY_MS"),
                    String::from("250"),
                ),
                (
                    String::from("HEPH_SERVICE_ISOLATION_CHECK"),
                    String::from("1"),
                ),
            ]),
            working_dir: Some(PathBuf::from("/")),
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        labels: BTreeMap::new(),
    }
}

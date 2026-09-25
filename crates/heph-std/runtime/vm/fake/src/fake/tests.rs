use super::{FakeProvider, PrivateHttpResponder};
use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
};
use tempfile::TempDir;
use vm_conformance::ProviderHarness;
use vm_trait::{
    DiskFormat, GuestCommand, NetworkMode, PortForward, PortProtocol, PrivateHttpRequest,
    PrivateHttpResponse, PrivateHttpServiceSpec, RootFilesystem, VmDisk, VmError, VmId, VmMount,
    VmProvider, VmResources, VmSpec,
};

struct FakeHarness {
    provider: Arc<FakeProvider>,
    temp: TempDir,
}

impl ProviderHarness for FakeHarness {
    fn provider(&self) -> Arc<dyn VmProvider> {
        self.provider.clone()
    }

    fn long_running_spec(&self, id: &str) -> VmSpec {
        spec(id)
    }

    fn ephemeral_ingress_spec(&self, id: &str) -> Option<VmSpec> {
        Some(spec(id))
    }

    fn caller_owned_spec(&self, id: &str) -> Option<VmSpec> {
        let root = self.temp.path().join("root");
        let disk = self.temp.path().join("agent-state.raw");
        let mount = self.temp.path().join("workspace");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&mount).unwrap();
        fs::write(&disk, b"persistent-state").unwrap();
        let mut requested = spec(id);
        requested.root = RootFilesystem::Directory { host_path: root };
        requested.disks.push(VmDisk {
            id: String::from("instance-state"),
            host_path: disk,
            format: DiskFormat::Raw,
            read_only: false,
        });
        requested.mounts.push(VmMount {
            tag: String::from("workspace"),
            host_path: mount,
            guest_path: PathBuf::from("/workspace"),
            read_only: false,
        });
        Some(requested)
    }
}

fn harness() -> FakeHarness {
    FakeHarness {
        provider: Arc::new(FakeProvider::new()),
        temp: TempDir::new().unwrap(),
    }
}

vm_conformance::provider_conformance_tests!(harness);

fn spec(id: &str) -> VmSpec {
    VmSpec {
        id: VmId(id.to_owned()),
        root: RootFilesystem::Directory {
            host_path: PathBuf::from("/fake/root"),
        },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 256,
        },
        network: NetworkMode::UserMode {
            ingress: vec![PortForward {
                protocol: PortProtocol::Tcp,
                bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                host_port: 0,
                guest_port: 22,
            }],
        },
        command: GuestCommand {
            program: "/bin/true".to_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/workspace")),
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        private_http_service: None,
        labels: BTreeMap::new(),
    }
}

#[tokio::test]
async fn invalid_working_directory_is_rejected() {
    let provider = FakeProvider::new();
    let mut invalid = spec("invalid");
    invalid.command.working_dir = Some(PathBuf::from("relative"));

    assert!(matches!(
        provider.provision(invalid).await,
        Err(VmError::InvalidSpec { field, .. }) if field == "command.working_dir"
    ));
}

#[tokio::test]
async fn private_http_service_requires_disabled_networking_and_bounded_values() {
    let provider = FakeProvider::new();
    for (name, service, field) in [
        (
            "port",
            PrivateHttpServiceSpec {
                loopback_port: 80,
                max_connections: 1,
                connect_timeout: std::time::Duration::from_secs(1),
            },
            "private_http_service.loopback_port",
        ),
        (
            "connections",
            PrivateHttpServiceSpec {
                loopback_port: 8080,
                max_connections: 65,
                connect_timeout: std::time::Duration::from_secs(1),
            },
            "private_http_service.max_connections",
        ),
        (
            "timeout",
            PrivateHttpServiceSpec {
                loopback_port: 8080,
                max_connections: 1,
                connect_timeout: std::time::Duration::from_millis(30_001),
            },
            "private_http_service.connect_timeout",
        ),
        (
            "precision",
            PrivateHttpServiceSpec {
                loopback_port: 8080,
                max_connections: 1,
                connect_timeout: std::time::Duration::from_nanos(1),
            },
            "private_http_service.connect_timeout",
        ),
    ] {
        let mut invalid = spec(&format!("private-http-invalid-{name}"));
        invalid.private_http_service = Some(service);
        assert!(matches!(
            provider.provision(invalid).await,
            Err(VmError::InvalidSpec { field: actual, .. }) if actual == field
        ));
    }

    let mut invalid_network = spec("private-http-network");
    invalid_network.network = NetworkMode::Disabled;
    invalid_network.private_http_service = Some(PrivateHttpServiceSpec {
        loopback_port: 8080,
        max_connections: 4,
        connect_timeout: std::time::Duration::from_secs(1),
    });
    invalid_network.network = NetworkMode::UserMode {
        ingress: Vec::new(),
    };

    assert!(matches!(
        provider.provision(invalid_network).await,
        Err(VmError::InvalidSpec { field, .. }) if field == "network"
    ));
}

struct EchoPrivateHttp;

#[async_trait]
impl PrivateHttpResponder for EchoPrivateHttp {
    async fn invoke(&self, request: PrivateHttpRequest) -> Result<PrivateHttpResponse, VmError> {
        Ok(PrivateHttpResponse {
            status: StatusCode::CREATED,
            headers: HeaderMap::new(),
            body: request.body,
            mailbox_publication: None,
        })
    }
}

#[tokio::test]
async fn private_http_works_without_guest_networking() {
    let provider = FakeProvider::new().with_private_http_responder(Arc::new(EchoPrivateHttp));
    let mut request_spec = spec("private-http");
    request_spec.network = NetworkMode::Disabled;
    let instance = provider.provision(request_spec).await.expect("provision");
    instance.start().await.expect("start");
    let response = instance
        .invoke_private_http(PrivateHttpRequest {
            method: http::Method::POST,
            path_and_query: "/gateway/echo".to_owned(),
            headers: HeaderMap::new(),
            body: Bytes::from_static(b"private"),
        })
        .await
        .expect("private handler response");
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.body, Bytes::from_static(b"private"));
    instance.destroy().await.expect("destroy");
}

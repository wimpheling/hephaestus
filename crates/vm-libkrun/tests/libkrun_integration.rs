//! Opt-in hardware integration tests for the Fedora libkrun backend.

use std::{
    collections::BTreeMap,
    env, fs, io,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vm_conformance::ProviderHarness;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    DiskFormat, GuestCommand, LogStream, NetworkMode, PortForward, PortProtocol, RootFilesystem,
    StopMode, VmDisk, VmError, VmEvent, VmId, VmMount, VmProvider, VmResources, VmSpec,
};

const ENABLE_FLAG: &str = "HEPHAESTUS_LIBKRUN_INTEGRATION";

#[tokio::test(flavor = "multi_thread")]
// Keeping the hardware scenarios sequential guarantees that they share no
// disk, cgroup, passt, or runtime fixture concurrently.
#[allow(clippy::too_many_lines)]
async fn boots_and_exercises_guest_runtime_without_privilege_escalation() {
    if env::var(ENABLE_FLAG).as_deref() != Ok("1") {
        return;
    }
    assert_ne!(
        rustix::process::geteuid().as_raw(),
        0,
        "hardware integration must not run as root"
    );

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
    let rootfs_for_graceful_test = rootfs.clone();
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
    let expected_limits = config.limits.clone();
    let mut timeout_config = config.clone();
    timeout_config.readiness_timeout = Duration::from_millis(100);
    let timeout_provider =
        LibkrunProvider::new(timeout_config).expect("timeout integration host configuration");
    let provider = LibkrunProvider::new(config).expect("integration host configuration");
    let spec = integration_spec(
        "integration-primary",
        rootfs.clone(),
        sqlite_disk.clone(),
        repository.clone(),
        workspace.clone(),
    );

    let vm = provider.provision(spec).await.expect("provision VM");
    let vm_id = vm.id().0.clone();
    let mut events = vm.subscribe_events();
    let first = Arc::clone(&vm);
    let second = Arc::clone(&vm);
    let (first, second) = tokio::join!(first.start(), second.start());
    first.expect("first concurrent start");
    second.expect("second concurrent start");
    assert_cgroup_limits(&cgroup_root_for_assertion.join(&vm_id), &expected_limits);

    let mut markers = String::new();
    let mut stderr_seen = false;
    let mut metric_seen = false;
    let mut ready_seen = false;
    let exit = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            match events.recv().await.expect("ordered VM event") {
                VmEvent::Started { ingress } => {
                    assert_ne!(ingress[0].host_port, 0);
                }
                VmEvent::Ready => ready_seen = true,
                VmEvent::Log { stream, bytes } => {
                    if matches!(stream, LogStream::Stderr) {
                        stderr_seen = true;
                    }
                    markers.push_str(&String::from_utf8_lossy(&bytes));
                }
                VmEvent::Metric(metric) if metric.name == "heph_init.ready" => {
                    metric_seen = true;
                }
                VmEvent::Exited(exit) => break exit,
                _ => {}
            }
        }
    })
    .await
    .expect("guest completion timeout");

    assert_eq!(exit.code, Some(0));
    for marker in [
        "sqlite=ok",
        "mounts=ok",
        "dns=ok",
        "tcp=ok",
        "udp=ok",
        "stderr=ok",
    ] {
        assert!(markers.contains(marker), "missing guest marker {marker}");
    }
    assert!(ready_seen, "missing Ready event");
    assert!(stderr_seen, "missing stderr event");
    assert!(metric_seen, "missing metric event");
    let previous_sqlite_rows = sqlite_previous_rows(&markers);
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

    let persisted = provider
        .provision(integration_spec(
            "integration-persistence",
            rootfs.clone(),
            sqlite_disk,
            repository,
            workspace,
        ))
        .await
        .expect("provision persistence VM");
    let persisted_id = persisted.id().0.clone();
    let mut persisted_events = persisted.subscribe_events();
    persisted.start().await.expect("start persistence VM");
    let persisted_markers = collect_logs_until_exit(&mut persisted_events).await;
    assert_eq!(
        sqlite_previous_rows(&persisted_markers),
        previous_sqlite_rows + 1,
        "SQLite contents did not persist across VM boots"
    );
    persisted.destroy().await.expect("destroy persistence VM");
    assert!(!runtime_root.join(&persisted_id).exists());
    assert!(!cgroup_root_for_assertion.join(&persisted_id).exists());

    let graceful = provider
        .provision(long_running_spec(rootfs_for_graceful_test, "graceful"))
        .await
        .expect("provision graceful-shutdown VM");
    let graceful_id = graceful.id().0.clone();
    graceful.start().await.expect("start graceful-shutdown VM");
    graceful
        .stop(StopMode::Graceful {
            timeout: Duration::from_secs(2),
        })
        .await
        .expect("guest accepts graceful cancellation");
    let graceful_exit = graceful.wait().await.expect("graceful exit is cached");
    assert_eq!(graceful_exit.signal, Some(15));
    graceful.destroy().await.expect("cleanup graceful VM");
    assert!(!runtime_root.join(&graceful_id).exists());
    assert!(!cgroup_root_for_assertion.join(&graceful_id).exists());

    let forced = provider
        .provision(long_running_spec(rootfs_for_force_test.clone(), "force"))
        .await
        .expect("provision forced-cleanup VM");
    let forced_id = forced.id().0.clone();
    forced.start().await.expect("start forced-cleanup VM");
    forced.destroy().await.expect("force cleanup running VM");
    let forced_exit = forced.wait().await.expect("forced exit is cached");
    assert!(forced_exit.signal.is_some() || forced_exit.code.is_some());
    assert!(!runtime_root.join(&forced_id).exists());
    assert!(!cgroup_root_for_assertion.join(&forced_id).exists());

    let disabled = provider
        .provision(mode_spec(
            rootfs.clone(),
            "integration-network-disabled",
            "--expect-network-disabled",
            NetworkMode::Disabled,
        ))
        .await
        .expect("provision disabled-network VM");
    let disabled_id = disabled.id().0.clone();
    let mut disabled_events = disabled.subscribe_events();
    disabled.start().await.expect("start disabled-network VM");
    let disabled_markers = collect_logs_until_exit(&mut disabled_events).await;
    assert!(disabled_markers.contains("network-disabled=ok"));
    disabled
        .destroy()
        .await
        .expect("destroy disabled-network VM");
    assert!(!runtime_root.join(&disabled_id).exists());
    assert!(!cgroup_root_for_assertion.join(&disabled_id).exists());

    let http = provider
        .provision(mode_spec(
            rootfs.clone(),
            "integration-http",
            "--serve-http",
            NetworkMode::UserMode {
                ingress: vec![PortForward {
                    protocol: PortProtocol::Tcp,
                    bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                    host_port: 0,
                    guest_port: 8080,
                }],
            },
        ))
        .await
        .expect("provision HTTP VM");
    let http_id = http.id().0.clone();
    let mut http_events = http.subscribe_events();
    http.start().await.expect("start HTTP VM");
    let host_port = ready_http_host_port(&mut http_events).await;
    let mut connection = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, host_port))
        .await
        .expect("connect through passt forwarding");
    connection
        .write_all(b"GET / HTTP/1.1\r\nHost: guest\r\nConnection: close\r\n\r\n")
        .await
        .expect("write forwarded HTTP request");
    let mut response = Vec::new();
    if let Err(error) = connection.read_to_end(&mut response).await {
        assert_eq!(
            error.kind(),
            io::ErrorKind::ConnectionReset,
            "read forwarded HTTP response: {error}"
        );
    }
    assert!(response.starts_with(b"HTTP/1.1 200 OK"));
    assert!(response.ends_with(b"\r\n\r\nok"));
    assert_eq!(http.wait().await.expect("HTTP guest exit").code, Some(0));
    http.destroy().await.expect("destroy HTTP VM");
    assert!(!runtime_root.join(&http_id).exists());
    assert!(!cgroup_root_for_assertion.join(&http_id).exists());

    let ignored = provider
        .provision(mode_spec(
            rootfs.clone(),
            "integration-ignore-cancellation",
            "--ignore-cancellation",
            NetworkMode::Disabled,
        ))
        .await
        .expect("provision non-cooperative VM");
    let ignored_id = ignored.id().0.clone();
    let mut ignored_events = ignored.subscribe_events();
    ignored.start().await.expect("start non-cooperative VM");
    wait_for_log(&mut ignored_events, "ignore-cancellation=ready").await;
    let grace = Duration::from_millis(150);
    let before_stop = Instant::now();
    ignored
        .stop(StopMode::Graceful { timeout: grace })
        .await
        .expect("force-stop non-cooperative guest after grace period");
    assert!(
        before_stop.elapsed() >= grace,
        "non-cooperative guest was killed before its grace period"
    );
    let ignored_exit = ignored.wait().await.expect("non-cooperative guest exit");
    assert!(ignored_exit.signal.is_some() || ignored_exit.code.is_some());
    ignored.destroy().await.expect("destroy non-cooperative VM");
    assert!(!runtime_root.join(&ignored_id).exists());
    assert!(!cgroup_root_for_assertion.join(&ignored_id).exists());

    let mut delayed_spec = long_running_spec(rootfs.clone(), "readiness-timeout");
    delayed_spec.command.env.insert(
        String::from("HEPH_TEST_READY_DELAY_MS"),
        String::from("1000"),
    );
    let delayed = timeout_provider
        .provision(delayed_spec)
        .await
        .expect("provision delayed-readiness VM");
    let delayed_id = delayed.id().0.clone();
    assert!(matches!(
        delayed.start().await,
        Err(VmError::Unavailable { resource, .. }) if resource == "guest readiness"
    ));
    delayed
        .destroy()
        .await
        .expect("destroy readiness-timeout VM");
    assert!(!runtime_root.join(&delayed_id).exists());
    assert!(!cgroup_root_for_assertion.join(&delayed_id).exists());

    let conformance = LibkrunHarness {
        provider: Arc::new(provider),
        rootfs: rootfs_for_force_test,
        runtime_root,
        cgroup_root: cgroup_root_for_assertion,
    };
    vm_conformance::lifecycle_suite(&conformance).await;
}

struct LibkrunHarness {
    provider: Arc<LibkrunProvider>,
    rootfs: PathBuf,
    runtime_root: PathBuf,
    cgroup_root: PathBuf,
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

fn integration_spec(
    id: &str,
    rootfs: PathBuf,
    sqlite_disk: PathBuf,
    repository: PathBuf,
    workspace: PathBuf,
) -> VmSpec {
    VmSpec {
        id: VmId(id.to_owned()),
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

fn mode_spec(rootfs: PathBuf, id: &str, argument: &str, network: NetworkMode) -> VmSpec {
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
        labels: BTreeMap::from([("test".to_owned(), id.to_owned())]),
    }
}

async fn collect_logs_until_exit(events: &mut tokio::sync::broadcast::Receiver<VmEvent>) -> String {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut logs = String::new();
        loop {
            match events.recv().await.expect("ordered VM event") {
                VmEvent::Log { bytes, .. } => {
                    logs.push_str(&String::from_utf8_lossy(&bytes));
                }
                VmEvent::Exited(exit) => {
                    assert_eq!(exit.code, Some(0));
                    return logs;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("guest completion timeout")
}

async fn ready_http_host_port(events: &mut tokio::sync::broadcast::Receiver<VmEvent>) -> u16 {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut host_port = None;
        let mut guest_ready = false;
        loop {
            match events.recv().await.expect("HTTP startup event") {
                VmEvent::Started { ingress } => {
                    assert_eq!(ingress.len(), 1);
                    host_port = Some(ingress[0].host_port);
                }
                VmEvent::Log { bytes, .. }
                    if String::from_utf8_lossy(&bytes).contains("http=ready") =>
                {
                    guest_ready = true;
                }
                _ => {}
            }
            if let (Some(port), true) = (host_port, guest_ready) {
                return port;
            }
        }
    })
    .await
    .expect("HTTP startup event timeout")
}

async fn wait_for_log(events: &mut tokio::sync::broadcast::Receiver<VmEvent>, expected: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let VmEvent::Log { bytes, .. } = events.recv().await.expect("guest log event")
                && String::from_utf8_lossy(&bytes).contains(expected)
            {
                return;
            }
        }
    })
    .await
    .expect("guest log marker timeout");
}

fn sqlite_previous_rows(markers: &str) -> u64 {
    markers
        .lines()
        .find_map(|line| line.strip_prefix("sqlite_previous="))
        .expect("missing sqlite_previous marker")
        .parse()
        .expect("invalid sqlite_previous marker")
}

fn required_path(name: &str) -> PathBuf {
    env::var_os(name).map_or_else(
        || panic!("{name} must be set when {ENABLE_FLAG}=1"),
        PathBuf::from,
    )
}

fn assert_cgroup_limits(path: &std::path::Path, limits: &vm_libkrun::CgroupLimits) {
    let cpu = limits.cpu_quota_micros.map_or_else(
        || format!("max {}", limits.cpu_period_micros),
        |quota| format!("{quota} {}", limits.cpu_period_micros),
    );
    assert_eq!(
        fs::read_to_string(path.join("cpu.max"))
            .expect("read worker CPU limit")
            .trim(),
        cpu
    );
    assert_eq!(
        fs::read_to_string(path.join("memory.max"))
            .expect("read worker memory limit")
            .trim(),
        limits.memory_max_bytes.to_string()
    );
    assert_eq!(
        fs::read_to_string(path.join("pids.max"))
            .expect("read worker PID limit")
            .trim(),
        limits.pids_max.to_string()
    );
    assert!(
        !fs::read_to_string(path.join("cgroup.procs"))
            .expect("read worker cgroup membership")
            .trim()
            .is_empty(),
        "worker cgroup contains no processes"
    );
}

fn long_running_spec(rootfs: PathBuf, kind: &str) -> VmSpec {
    VmSpec {
        id: VmId(format!("integration-{kind}-{}", std::process::id())),
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
        labels: BTreeMap::from([("test".to_owned(), kind.to_owned())]),
    }
}

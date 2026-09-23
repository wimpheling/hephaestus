//! Opt-in hardware integration tests for the Fedora libkrun backend.

use bytes::Bytes;
use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use gateway_edge::{
    GatewayEdgeError, GatewayRequest, GatewayScheme, GatewayServiceFailure,
    GatewayServiceFailureCode, GatewayServiceIdentity, GatewayServiceLaunch,
    GatewayServiceLaunchRequest, GatewayServiceLaunchResolver, ServiceHttpPolicy,
    ServiceInstancePolicy, ServiceProbePolicy, ServiceWorkerState, TrustedRequestMetadata,
    exchange_private_service_http, new_service_instance, probe_private_service_http,
};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use runtime_types::RunId;
use secret_broker::{
    BrokerExecutor, BrokerServer, WireBrokerRequest, WireBrokerResponse, WireBrokerStatus,
};
use secret_domain::{SecretSlotKey, SecretValue};
use secret_runtime::{EphemeralSecretConfig, RawSecretFile, materialize};
use std::{
    collections::BTreeMap,
    env, fs, io,
    net::{IpAddr, Ipv4Addr},
    os::unix::fs::PermissionsExt,
    os::unix::net::UnixListener,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vm_conformance::ProviderHarness;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    DiskFormat, GuestCommand, LogStream, NetworkMode, PortForward, PortProtocol,
    PrivateHttpRequest, PrivateHttpServiceSpec, RUNTIME_AUTHORITY_CREDENTIAL_BYTES,
    RUNTIME_GIT_CREDENTIAL_BYTES, RootFilesystem, RuntimeAuthorityBootstrap, RuntimeGitBridge,
    StopMode, VmDisk, VmError, VmEvent, VmId, VmInstance, VmMount, VmProvider, VmResources, VmSpec,
};

const ENABLE_FLAG: &str = "HEPHAESTUS_LIBKRUN_INTEGRATION";
static INTEGRATION_TEST_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

/// Host fixture that proves the released guest client forwarded only the exact
/// runtime bearer and canonical broker request across the private vsock path.
struct IntegrationBroker {
    credential: [u8; RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
    session_id: uuid::Uuid,
}

struct IntegrationServiceResolver {
    expected: GatewayServiceIdentity,
    cleanups: AtomicUsize,
}

#[async_trait::async_trait]
impl GatewayServiceLaunchResolver for IntegrationServiceResolver {
    async fn resolve_service_launch(
        &self,
        _: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }

    async fn cleanup_service_launch(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        assert_eq!(identity, self.expected);
        self.cleanups.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[async_trait::async_trait]
impl BrokerExecutor for IntegrationBroker {
    async fn execute(&self, request: WireBrokerRequest) -> WireBrokerResponse {
        assert_eq!(request.credential, self.credential);
        assert_eq!(request.run_id.as_uuid(), self.session_id);
        assert_eq!(request.slot, "model");
        assert_eq!(request.destination, "api.example.test");
        assert_eq!(request.operation, "https_v1");
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("canonical brokered HTTPS body");
        assert_eq!(body["rule_id"], "00000000-0000-0000-0000-000000000002");
        assert_eq!(body["method"], "get");
        assert_eq!(body["path_and_query"], "/v1/probe");
        assert_eq!(body["headers"], serde_json::json!([]));
        assert_eq!(body["body"], serde_json::json!([]));
        WireBrokerResponse {
            status: WireBrokerStatus::Succeeded,
            body: b"ok".to_vec(),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
// Keeping the hardware scenarios sequential guarantees that they share no
// disk, cgroup, passt, or runtime fixture concurrently.
#[allow(clippy::too_many_lines)]
async fn boots_and_exercises_guest_runtime_without_privilege_escalation() {
    if env::var(ENABLE_FLAG).as_deref() != Ok("1") {
        return;
    }
    let _test_lock = INTEGRATION_TEST_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
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
    let sqlite_disk_for_assertion = sqlite_disk.clone();
    let sqlite_uuid = required_text("HEPHAESTUS_LIBKRUN_SQLITE_UUID");
    let mount_root = required_path("HEPHAESTUS_LIBKRUN_MOUNT_ROOT");
    let secret_root = mount_root.join("secrets");
    fs::create_dir(&secret_root).expect("create secret mount root");
    fs::set_permissions(&secret_root, fs::Permissions::from_mode(0o700))
        .expect("protect secret mount root");
    let mut secret_mount = materialize(
        &EphemeralSecretConfig {
            root: secret_root,
            require_memory_filesystem: false,
        },
        RunId::new(),
        vec![RawSecretFile {
            slot: SecretSlotKey::parse("model").expect("secret slot"),
            value: SecretValue::new("libkrun-secret-sentinel-8a4c").expect("secret fixture value"),
        }],
    )
    .expect("materialize exact raw secret fixture");
    let secret_path = secret_mount.host_path().to_path_buf();
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
        vec![mount_root.clone()],
        env!("CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"),
        cgroup_root,
    );
    let broker_socket = mount_root.join("brokered-egress.sock");
    let broker_credential = [0xA5; RUNTIME_AUTHORITY_CREDENTIAL_BYTES];
    let broker_session = uuid::Uuid::new_v4();
    let broker_server = BrokerServer::bind(
        &broker_socket,
        Arc::new(IntegrationBroker {
            credential: broker_credential,
            session_id: broker_session,
        }),
    )
    .expect("bind private broker socket");
    let broker_shutdown = tokio_util::sync::CancellationToken::new();
    let broker_task = tokio::spawn(broker_server.serve(broker_shutdown.clone()));
    config.broker_socket_path = Some(broker_socket);
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
        &sqlite_uuid,
        repository.clone(),
        workspace.clone(),
        Some(secret_mount.vm_mount()),
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

    assert_eq!(exit.code, Some(0), "guest output:\n{markers}");
    for marker in [
        "sqlite=ok",
        "mounts=ok",
        "secrets=ok",
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
    secret_mount
        .mark_guest_destroyed()
        .expect("confirm primary guest destruction");
    secret_mount
        .destroy()
        .expect("destroy exact raw secret mount");
    assert!(
        !secret_path.exists(),
        "raw secret mount survived guest destruction"
    );
    assert!(!runtime_root.join(&vm_id).exists());
    assert!(!cgroup_root_for_assertion.join(&vm_id).exists());

    let rollback = provider
        .provision(state_probe_spec(
            "integration-state-rollback",
            rootfs.clone(),
            sqlite_disk.clone(),
            &sqlite_uuid,
            "--state-rollback",
        ))
        .await
        .expect("provision rollback update VM");
    let rollback_id = rollback.id().0.clone();
    let mut rollback_events = rollback.subscribe_events();
    rollback.start().await.expect("start rollback update VM");
    let (rollback_markers, rollback_exit) = collect_logs_until_any_exit(&mut rollback_events).await;
    assert_eq!(rollback_exit.code, Some(23));
    assert!(rollback_markers.contains("sqlite-rollback=ok"));
    rollback
        .destroy()
        .await
        .expect("destroy rollback update VM");
    assert!(!runtime_root.join(&rollback_id).exists());
    assert!(!cgroup_root_for_assertion.join(&rollback_id).exists());

    let persisted = provider
        .provision(integration_spec(
            "integration-persistence",
            rootfs.clone(),
            sqlite_disk,
            &sqlite_uuid,
            repository,
            workspace,
            None,
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
    assert!(
        sqlite_disk_for_assertion.is_file(),
        "destroy removed caller-owned state disk"
    );
    assert!(!runtime_root.join(&persisted_id).exists());

    let gateway = provider
        .provision(private_http_spec(rootfs.clone()))
        .await
        .expect("provision private HTTP gateway VM");
    let gateway_id = gateway.id().0.clone();
    gateway
        .start()
        .await
        .expect("start private HTTP gateway VM");
    let mut request_headers = HeaderMap::new();
    request_headers.insert("content-type", HeaderValue::from_static("text/plain"));
    let response = gateway
        .invoke_private_http(PrivateHttpRequest {
            method: Method::POST,
            path_and_query: "/gateway/proof?mode=real".to_owned(),
            headers: request_headers,
            body: Bytes::from_static(b"gateway-real-vm-request"),
        })
        .await
        .expect("complete private HTTP request through real guest control channel");
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(
        response.headers.get("content-type"),
        Some(&HeaderValue::from_static("text/plain"))
    );
    assert_eq!(
        response.body,
        Bytes::from_static(b"gateway-real-vm-response")
    );
    gateway
        .destroy()
        .await
        .expect("destroy private HTTP gateway VM");
    assert!(!runtime_root.join(&gateway_id).exists());
    assert!(!cgroup_root_for_assertion.join(&gateway_id).exists());
    assert!(!cgroup_root_for_assertion.join(&persisted_id).exists());

    let service_spec = private_service_spec(
        rootfs.clone(),
        format!("integration-private-service-{}", std::process::id()),
    );
    assert!(service_spec.runtime_authority.is_none());
    assert!(matches!(service_spec.network, NetworkMode::Disabled));
    assert!(
        !service_spec
            .labels
            .contains_key("hephaestus.gateway.handler-contract")
    );
    let service = provider
        .provision(service_spec)
        .await
        .expect("provision persistent private service VM");
    let service_id = service.id().0.clone();
    let mut service_events = service.subscribe_events();
    service
        .start()
        .await
        .expect("start persistent private service VM");
    wait_for_service_isolation(&mut service_events).await;

    let ready = poll_private_service(&service, "/readyz").await;
    assert_eq!(ready.0, 200, "service readiness response: {ready:?}");
    assert_eq!(ready.1, b"ready");
    assert_eq!(
        poll_private_service(&service, "/healthz").await,
        (200, b"healthy".to_vec())
    );

    let readiness_probe = poll_private_service_probe(&service, "/readyz").await;
    assert_eq!(readiness_probe.status, StatusCode::OK);
    let health_probe = poll_private_service_probe(&service, "/healthz").await;
    assert_eq!(health_probe.status, StatusCode::OK);

    let identity_one = parse_adapter_service_identity(
        &private_service_adapter_request(&service, "/identity").await,
    );
    let identity_two = parse_adapter_service_identity(
        &private_service_adapter_request(&service, "/identity").await,
    );
    assert_eq!(identity_one["pid"], identity_two["pid"]);
    assert_eq!(identity_one["startup_id"], identity_two["startup_id"]);

    let delayed_vm = Arc::clone(&service);
    let delayed =
        tokio::spawn(async move { private_service_request(&delayed_vm, "/delay/1500").await });
    wait_for_log(&mut service_events, "private-service-delay=started").await;
    let health = tokio::time::timeout(
        Duration::from_secs(1),
        private_service_request(&service, "/healthz"),
    )
    .await
    .expect("health request did not complete while delayed request was active")
    .expect("concurrent health response");
    assert_eq!(health, (200, b"healthy".to_vec()));
    assert!(
        !delayed.is_finished(),
        "delayed request completed too early"
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), delayed)
            .await
            .expect("delayed service response timeout")
            .expect("delayed service task")
            .expect("delayed service response")
            .0,
        200
    );

    let held_one = service
        .open_private_service_connection()
        .await
        .expect("first service capacity connection");
    let held_two = service
        .open_private_service_connection()
        .await
        .expect("second service capacity connection");
    assert!(matches!(
        service.open_private_service_connection().await,
        Err(VmError::Unavailable { .. })
    ));
    drop(held_one);
    drop(held_two);

    let active = service
        .open_private_service_connection()
        .await
        .expect("active service connection before destroy");
    service
        .destroy()
        .await
        .expect("destroy persistent service VM");
    let mut active = active;
    let mut closed = [0_u8; 1];
    let close_result = tokio::time::timeout(Duration::from_secs(5), active.read(&mut closed))
        .await
        .expect("destroy closes active service stream");
    assert!(matches!(close_result, Err(_) | Ok(0)));
    assert!(!runtime_root.join(&service_id).exists());
    assert!(!cgroup_root_for_assertion.join(&service_id).exists());

    let worker_identity = GatewayServiceIdentity {
        instance_id: uuid::Uuid::new_v4(),
        gateway_id: uuid::Uuid::new_v4(),
        revision_id: uuid::Uuid::new_v4(),
    };
    let worker_spec = private_service_spec(
        rootfs.clone(),
        format!("gateway-service-{}", worker_identity.instance_id),
    );
    let worker_launch = GatewayServiceLaunch {
        identity: worker_identity,
        service: GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/readyz").expect("worker readiness path"),
            ServiceProbePath::parse("/healthz").expect("worker health path"),
        )
        .expect("worker service declaration"),
        spec: worker_spec,
    };
    let worker_vm = provider
        .provision(worker_launch.spec.clone())
        .await
        .expect("provision prepared service worker VM");
    let worker_vm_id = worker_vm.id().0.clone();
    let worker_resolver = Arc::new(IntegrationServiceResolver {
        expected: worker_identity,
        cleanups: AtomicUsize::new(0),
    });
    let (worker_handle, mut worker_state, worker) = new_service_instance(
        worker_launch,
        Arc::clone(&worker_vm),
        worker_resolver.clone(),
        "service.internal",
        ServiceInstancePolicy::new(
            Duration::from_secs(30),
            Duration::from_millis(100),
            Duration::from_secs(3),
            Duration::from_secs(5),
        ),
    )
    .expect("construct prepared service worker");
    let worker_task = tokio::spawn(worker.run());
    tokio::time::timeout(Duration::from_secs(60), async {
        while *worker_state.borrow() != ServiceWorkerState::Ready {
            worker_state
                .changed()
                .await
                .expect("prepared service worker state");
        }
    })
    .await
    .expect("prepared service worker readiness timeout");
    println!("REAL_PREPARED_SERVICE_WORKER_READY=1");
    assert_eq!(
        worker_handle
            .health()
            .await
            .expect("prepared service worker health"),
        StatusCode::OK
    );
    println!("REAL_PREPARED_SERVICE_WORKER_HEALTH=1");
    let first_identity = parse_adapter_service_identity(
        &private_service_adapter_request(&worker_vm, "/identity").await,
    );
    assert_eq!(
        private_service_request(&worker_vm, "/crash")
            .await
            .expect("prepared service worker crash response"),
        (503, b"crashing".to_vec())
    );
    let worker_result = tokio::time::timeout(Duration::from_secs(15), worker_task)
        .await
        .expect("prepared service worker cleanup timeout")
        .expect("prepared service worker join")
        .expect_err("prepared service worker exit must be reported");
    assert_eq!(
        worker_result,
        gateway_edge::ServiceInstanceError::UnexpectedExit
    );
    assert_eq!(
        worker_handle.failure(),
        Some(GatewayServiceFailure {
            code: GatewayServiceFailureCode::UnexpectedExit,
            exit_code: Some(42),
            exit_signal: None,
        })
    );
    assert_eq!(worker_resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(!runtime_root.join(&worker_vm_id).exists());
    assert!(!cgroup_root_for_assertion.join(&worker_vm_id).exists());
    println!("REAL_PREPARED_SERVICE_WORKER_CRASH=1");
    println!("REAL_PREPARED_SERVICE_WORKER_CLEANED=1");

    let replacement_identity = GatewayServiceIdentity {
        instance_id: uuid::Uuid::new_v4(),
        ..worker_identity
    };
    let replacement_launch = GatewayServiceLaunch {
        identity: replacement_identity,
        service: GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/readyz").expect("replacement readiness path"),
            ServiceProbePath::parse("/healthz").expect("replacement health path"),
        )
        .expect("replacement service declaration"),
        spec: private_service_spec(
            rootfs.clone(),
            format!("gateway-service-{}", replacement_identity.instance_id),
        ),
    };
    let replacement_vm = provider
        .provision(replacement_launch.spec.clone())
        .await
        .expect("provision replacement service worker VM");
    let replacement_vm_id = replacement_vm.id().0.clone();
    assert_ne!(replacement_vm_id, worker_vm_id);
    let replacement_resolver = Arc::new(IntegrationServiceResolver {
        expected: replacement_identity,
        cleanups: AtomicUsize::new(0),
    });
    let (replacement_handle, mut replacement_state, replacement_worker) = new_service_instance(
        replacement_launch,
        Arc::clone(&replacement_vm),
        replacement_resolver.clone(),
        "service.internal",
        ServiceInstancePolicy::new(
            Duration::from_secs(30),
            Duration::from_millis(100),
            Duration::from_secs(3),
            Duration::from_secs(5),
        ),
    )
    .expect("construct replacement service worker");
    let replacement_task = tokio::spawn(replacement_worker.run());
    tokio::time::timeout(Duration::from_secs(60), async {
        while *replacement_state.borrow() != ServiceWorkerState::Ready {
            replacement_state
                .changed()
                .await
                .expect("replacement service worker state");
        }
    })
    .await
    .expect("replacement service worker readiness timeout");
    let replacement_identity_response = parse_adapter_service_identity(
        &private_service_adapter_request(&replacement_vm, "/identity").await,
    );
    assert_ne!(
        first_identity["startup_id"],
        replacement_identity_response["startup_id"]
    );
    replacement_handle.shutdown();
    tokio::time::timeout(Duration::from_secs(15), replacement_task)
        .await
        .expect("replacement service worker cleanup timeout")
        .expect("replacement service worker join")
        .expect("replacement service worker cleanup");
    assert_eq!(replacement_handle.failure(), None);
    assert_eq!(replacement_resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(!runtime_root.join(&replacement_vm_id).exists());
    assert!(!cgroup_root_for_assertion.join(&replacement_vm_id).exists());
    println!("REAL_PREPARED_SERVICE_WORKER_REPLACED=1");

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

    let mut forced_secret_mount = materialize(
        &EphemeralSecretConfig {
            root: secret_path
                .parent()
                .expect("primary secret mount has a parent")
                .to_path_buf(),
            require_memory_filesystem: false,
        },
        RunId::new(),
        vec![RawSecretFile {
            slot: SecretSlotKey::parse("forced_model").expect("forced secret slot"),
            value: SecretValue::new("libkrun-forced-secret-sentinel-91bd")
                .expect("forced secret fixture value"),
        }],
    )
    .expect("materialize forced-cleanup raw secret fixture");
    let forced_secret_path = forced_secret_mount.host_path().to_path_buf();
    let mut forced_spec = long_running_spec(rootfs_for_force_test.clone(), "force");
    forced_spec.mounts.push(forced_secret_mount.vm_mount());
    let forced = provider
        .provision(forced_spec)
        .await
        .expect("provision forced-cleanup VM");
    let forced_id = forced.id().0.clone();
    forced.start().await.expect("start forced-cleanup VM");
    forced.destroy().await.expect("force cleanup running VM");
    let forced_exit = forced.wait().await.expect("forced exit is cached");
    assert!(forced_exit.signal.is_some() || forced_exit.code.is_some());
    forced_secret_mount
        .mark_guest_destroyed()
        .expect("confirm forced guest destruction");
    forced_secret_mount
        .destroy()
        .expect("destroy forced-cleanup raw secret mount");
    assert!(
        !forced_secret_path.exists(),
        "raw secret mount survived forced guest destruction"
    );
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

    // This is the VM half of the durable mailbox journey: the same sealed
    // control mount produced by `run-runtime-local` reaches a real libkrun
    // guest as a generic envelope plus exact opaque body, never via NATS.
    let mailbox_root = mount_root.join("mailbox-control");
    fs::create_dir(&mailbox_root).expect("create mailbox control root");
    fs::write(
        mailbox_root.join("mailbox-event.json"),
        r#"{"schema_version":1,"method":"POST","route":"/mailbox/libkrun-proof","body_path":"/run/hephaestus/mailbox-body"}"#,
    )
    .expect("write mailbox envelope");
    fs::write(
        mailbox_root.join("mailbox-body"),
        b"real-libkrun-mailbox-body",
    )
    .expect("write opaque mailbox body");
    fs::set_permissions(&mailbox_root, fs::Permissions::from_mode(0o555))
        .expect("seal mailbox control root");
    let mut mailbox_spec = mode_spec(
        rootfs.clone(),
        "integration-mailbox-control",
        "--expect-mailbox",
        NetworkMode::Disabled,
    );
    mailbox_spec.mounts.push(VmMount {
        tag: "mailbox-control".to_owned(),
        host_path: mailbox_root,
        guest_path: PathBuf::from("/run/hephaestus"),
        read_only: true,
    });
    let mailbox = provider
        .provision(mailbox_spec)
        .await
        .expect("provision mailbox control VM");
    let mailbox_id = mailbox.id().0.clone();
    let mut mailbox_events = mailbox.subscribe_events();
    mailbox.start().await.expect("start mailbox control VM");
    assert!(
        collect_logs_until_exit(&mut mailbox_events)
            .await
            .contains("mailbox=ok")
    );
    mailbox.destroy().await.expect("destroy mailbox control VM");
    assert!(!runtime_root.join(&mailbox_id).exists());
    assert!(!cgroup_root_for_assertion.join(&mailbox_id).exists());

    let mut broker_spec = mode_spec(
        rootfs.clone(),
        "integration-broker-only-no-ip-bypass",
        "--expect-broker-only",
        NetworkMode::BrokerOnly,
    );
    broker_spec.runtime_authority = Some(RuntimeAuthorityBootstrap::new(
        broker_session,
        1,
        broker_credential,
    ));
    let broker_only = provider
        .provision(broker_spec)
        .await
        .expect("provision broker-only VM");
    let broker_only_id = broker_only.id().0.clone();
    let mut broker_only_events = broker_only.subscribe_events();
    broker_only.start().await.expect("start broker-only VM");
    let broker_only_markers = collect_logs_until_exit(&mut broker_only_events).await;
    assert!(broker_only_markers.contains("network-disabled=ok"));
    assert!(broker_only_markers.contains("broker-vsock=ok"));
    broker_shutdown.cancel();
    broker_task
        .await
        .expect("broker server task")
        .expect("broker server exit");
    broker_only.destroy().await.expect("destroy broker-only VM");
    assert!(!runtime_root.join(&broker_only_id).exists());
    assert!(!cgroup_root_for_assertion.join(&broker_only_id).exists());

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

/// Exercises the actual guest loopback proxy, libkrun's guest-to-host vsock
/// mapping, and a host Unix stream peer. The peer is a bounded fake HTTP
/// endpoint for transport coverage; it does not assert production Git auth.
#[tokio::test(flavor = "multi_thread")]
// This test keeps fixture setup, host peer, VM lifecycle, and exact response
// assertions together so the real transport path remains auditable.
#[allow(clippy::too_many_lines)]
async fn real_guest_runtime_git_bridge_forwards_disabled_network_http() {
    if env::var(ENABLE_FLAG).as_deref() != Ok("1") {
        return;
    }
    let _test_lock = INTEGRATION_TEST_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    assert_ne!(rustix::process::geteuid().as_raw(), 0);

    let runtime_root = required_path("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT");
    let image_root = required_path("HEPHAESTUS_LIBKRUN_IMAGE_ROOT");
    let rootfs = required_path("HEPHAESTUS_LIBKRUN_ROOTFS");
    let disk_root = required_path("HEPHAESTUS_LIBKRUN_DISK_ROOT");
    let mount_root = required_path("HEPHAESTUS_LIBKRUN_MOUNT_ROOT");
    let cgroup_root = required_path("HEPHAESTUS_LIBKRUN_CGROUP_ROOT");
    let socket_path = mount_root.join(format!("runtime-git-{}.sock", std::process::id()));
    let repository_id = uuid::Uuid::new_v4();
    let listener = UnixListener::bind(&socket_path).expect("bind runtime Git bridge socket");
    listener
        .set_nonblocking(true)
        .expect("make runtime Git listener cancellable");
    let host_task = thread::spawn(move || -> Result<(), String> {
        let connection = (0..600)
            .find_map(|_| match listener.accept() {
                Ok(connection) => Some(Ok(connection)),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                    None
                }
                Err(error) => Some(Err(error)),
            })
            .ok_or_else(|| String::from("runtime Git host listener timed out"))?;
        let (mut stream, _) = connection.map_err(|error| error.to_string())?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read =
                std::io::Read::read(&mut stream, &mut buffer).map_err(|error| error.to_string())?;
            if read == 0 || request.len() + read > 64 * 1024 {
                return Err(String::from(
                    "runtime Git request was truncated or oversized",
                ));
            }
            request.extend_from_slice(&buffer[..read]);
        }
        if !request
            .starts_with(format!("GET /{repository_id}/runtime-git-proof HTTP/1.1").as_bytes())
        {
            return Err(String::from("runtime Git request route was not exact"));
        }
        if !request
            .windows(b"Authorization: Basic ".len())
            .any(|window| window == b"Authorization: Basic ")
        {
            return Err(String::from("runtime Git request omitted authorization"));
        }
        std::io::Write::write_all(
            &mut stream,
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 20\r\nConnection: close\r\n\r\nruntime-git-response",
        )
        .map_err(|error| error.to_string())?;
        thread::sleep(Duration::from_millis(250));
        Ok(())
    });

    let mut config = LibkrunConfig::new(
        &runtime_root,
        vec![image_root],
        vec![disk_root],
        vec![mount_root],
        env!("CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"),
        cgroup_root,
    );
    config.runtime_git_socket_path = Some(socket_path.clone());
    config.startup_timeout = Duration::from_secs(15);
    config.readiness_timeout = Duration::from_secs(45);
    let provider = LibkrunProvider::new(config).expect("runtime Git integration configuration");
    let session_id = uuid::Uuid::new_v4();
    let authority = [0x31; RUNTIME_AUTHORITY_CREDENTIAL_BYTES];
    let git_credential = [0x32; RUNTIME_GIT_CREDENTIAL_BYTES];
    let spec = VmSpec {
        id: VmId(format!("integration-runtime-git-{}", std::process::id())),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: String::from("/usr/libexec/hephaestus/integration-check"),
            args: vec![
                String::from("--runtime-git-http"),
                repository_id.to_string(),
            ],
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/")),
        },
        runtime_authority: Some(
            RuntimeAuthorityBootstrap::new(session_id, 1, authority)
                .with_runtime_git_credential(git_credential),
        ),
        private_http_service: None,
        runtime_git_bridge: Some(RuntimeGitBridge::new(repository_id, 19_100)),
        labels: BTreeMap::new(),
    };
    let vm = provider
        .provision(spec)
        .await
        .expect("provision runtime Git bridge VM");
    let vm_id = vm.id().0.clone();
    let mut events = vm.subscribe_events();
    vm.start().await.expect("start runtime Git bridge VM");
    let exit = vm.wait().await.expect("wait runtime Git bridge VM");
    let host_result = host_task.join().expect("runtime Git host thread");
    let mut logs = String::new();
    while let Ok(event) = events.try_recv() {
        if let VmEvent::Log { bytes, .. } = event {
            logs.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    host_result.expect("runtime Git host exchange");
    assert_eq!(exit.code, Some(0), "guest bridge logs: {logs}");
    vm.destroy().await.expect("destroy runtime Git bridge VM");
    assert!(!runtime_root.join(vm_id).exists());
    fs::remove_file(&socket_path).expect("remove test-owned runtime Git socket");
    assert!(!socket_path.exists());
}

fn state_probe_spec(
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

fn integration_spec(
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
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::from([("test".to_owned(), id.to_owned())]),
    }
}

fn private_http_spec(rootfs: PathBuf) -> VmSpec {
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

fn private_service_spec(rootfs: PathBuf, id: String) -> VmSpec {
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

async fn poll_private_service(vm: &Arc<dyn VmInstance>, path: &str) -> (u16, Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match private_service_request(vm, path).await {
                Ok(response) if response.0 == 200 => return response,
                Ok(_) | Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .expect("private service readiness polling timeout")
}

async fn poll_private_service_probe(
    vm: &Arc<dyn VmInstance>,
    path: &str,
) -> gateway_edge::ServiceProbeSuccess {
    let path = ServiceProbePath::parse(path).expect("declared service probe path");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match probe_private_service_http(
                vm.as_ref(),
                &path,
                "service.internal",
                ServiceProbePolicy::new(Duration::from_secs(5)),
            )
            .await
            {
                Ok(response) => return response,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .expect("private service probe readiness timeout")
}

async fn private_service_adapter_request(
    vm: &Arc<dyn VmInstance>,
    path: &str,
) -> gateway_edge::GatewayResponse {
    let connection = vm
        .open_private_service_connection()
        .await
        .expect("open service connection for gateway-edge adapter");
    exchange_private_service_http(
        connection,
        GatewayRequest {
            method: Method::GET,
            path_and_query: path.to_owned(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Http,
                authority: String::from("service.internal"),
                client_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
                request_id: uuid::Uuid::new_v4(),
            },
        },
        ServiceHttpPolicy {
            max_request_body_bytes: 1,
            max_response_body_bytes: 64 * 1024,
            max_request_headers: 4,
            max_response_headers: 32,
            max_path_and_query_bytes: 512,
            max_wire_header_bytes: 8 * 1024,
            exchange_timeout: Duration::from_secs(5),
        },
    )
    .await
    .expect("gateway-edge private service exchange")
}

async fn private_service_request(
    vm: &Arc<dyn VmInstance>,
    path: &str,
) -> io::Result<(u16, Vec<u8>)> {
    tokio::time::timeout(
        Duration::from_secs(5),
        private_service_request_inner(vm, path),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "private service request timeout"))?
}

async fn private_service_request_inner(
    vm: &Arc<dyn VmInstance>,
    path: &str,
) -> io::Result<(u16, Vec<u8>)> {
    const MAX_SERVICE_RESPONSE_BYTES: usize = 64 * 1024;
    let mut stream = vm
        .open_private_service_connection()
        .await
        .map_err(|error| io::Error::other(error.to_string()))?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: guest\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await?;
    let mut response = Vec::new();
    let mut limited = stream.take((MAX_SERVICE_RESPONSE_BYTES + 1) as u64);
    limited.read_to_end(&mut response).await?;
    if response.len() > MAX_SERVICE_RESPONSE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private service response exceeds test limit",
        ));
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "service response has no headers",
            )
        })?;
    let status = std::str::from_utf8(&response[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "service response is not UTF-8"))?
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "service response has no status")
        })?
        .parse::<u16>()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "service response status is invalid",
            )
        })?;
    Ok((status, response[header_end + 4..].to_vec()))
}

fn parse_adapter_service_identity(response: &gateway_edge::GatewayResponse) -> serde_json::Value {
    assert_eq!(response.status, StatusCode::OK);
    serde_json::from_slice(&response.body).expect("service identity JSON")
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

async fn collect_logs_until_any_exit(
    events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
) -> (String, vm_trait::VmExit) {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut logs = String::new();
        loop {
            match events.recv().await.expect("ordered VM event") {
                VmEvent::Log { bytes, .. } => {
                    logs.push_str(&String::from_utf8_lossy(&bytes));
                }
                VmEvent::Exited(exit) => return (logs, exit),
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
            if let VmEvent::Log { bytes, .. } = events.recv().await.expect("guest log event") {
                if String::from_utf8_lossy(&bytes).contains(expected) {
                    return;
                }
            }
        }
    })
    .await
    .expect("guest log marker timeout");
}

async fn wait_for_service_isolation(events: &mut tokio::sync::broadcast::Receiver<VmEvent>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut started = false;
        loop {
            match events.recv().await.expect("service guest event") {
                VmEvent::Started { ingress } => {
                    assert!(ingress.is_empty(), "private service received ingress");
                    started = true;
                }
                VmEvent::Log { bytes, .. }
                    if String::from_utf8_lossy(&bytes).contains("private-service-isolation=ok") =>
                {
                    assert!(started, "service isolation marker preceded VM start event");
                    return;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("service isolation marker timeout");
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

fn required_text(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} must be set when {ENABLE_FLAG}=1"))
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
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::from([("test".to_owned(), kind.to_owned())]),
    }
}

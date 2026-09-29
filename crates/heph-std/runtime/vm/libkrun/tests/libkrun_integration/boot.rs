use std::{env, fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc, time::Duration};

use runtime_types::RunId;
use secret_broker::BrokerServer;
use secret_domain::{SecretSlotKey, SecretValue};
use secret_runtime::{EphemeralSecretConfig, EphemeralSecretMount, RawSecretFile, materialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};

use crate::{ENABLE_FLAG, INTEGRATION_TEST_LOCK, support::*};

#[path = "boot/modes.rs"]
mod modes;
#[path = "boot/network.rs"]
mod network;
#[path = "boot/primary.rs"]
mod primary;
#[path = "boot/service.rs"]
mod service;

pub struct BootContext {
    pub runtime_root: PathBuf,
    pub rootfs: PathBuf,
    pub sqlite_disk: PathBuf,
    pub sqlite_disk_for_assertion: PathBuf,
    pub sqlite_uuid: String,
    pub mount_root: PathBuf,
    pub repository: PathBuf,
    pub workspace: PathBuf,
    pub cgroup_root: PathBuf,
    pub rootfs_for_graceful_test: PathBuf,
    pub rootfs_for_force_test: PathBuf,
    pub expected_limits: vm_libkrun::CgroupLimits,
    pub timeout_provider: LibkrunProvider,
    pub provider: LibkrunProvider,
    pub secret_mount: Option<EphemeralSecretMount>,
    pub secret_path: PathBuf,
    pub broker_credential: [u8; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
    pub broker_session: uuid::Uuid,
    pub broker_shutdown: Option<CancellationToken>,
    pub broker_task: Option<JoinHandle<Result<(), secret_broker::BrokerServerError>>>,
}

#[tokio::test(flavor = "multi_thread")]
// Keeping the hardware scenarios sequential guarantees that they share no
// disk, cgroup, passt, or runtime fixture concurrently.
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
    let mut context = setup();
    primary::run(&mut context).await;
    service::run(&context).await;
    modes::run(&context).await;
    network::run(&mut context).await;
    let conformance = LibkrunHarness {
        provider: Arc::new(context.provider),
        rootfs: context.rootfs_for_force_test,
        runtime_root: context.runtime_root,
        cgroup_root: context.cgroup_root,
    };
    vm_conformance::lifecycle_suite(&conformance).await;
}

fn setup() -> BootContext {
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
    let secret_mount = materialize(
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
    let rootfs_for_graceful_test = rootfs.clone();
    let rootfs_for_force_test = rootfs.clone();
    let mut config = LibkrunConfig::new(
        &runtime_root,
        vec![image_root],
        vec![disk_root],
        vec![mount_root.clone()],
        env!("CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"),
        cgroup_root.clone(),
    );
    let broker_socket = mount_root.join("brokered-egress.sock");
    let broker_credential = [0xA5; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES];
    let broker_session = uuid::Uuid::new_v4();
    let broker_server = BrokerServer::bind(
        &broker_socket,
        Arc::new(IntegrationBroker {
            credential: broker_credential,
            session_id: broker_session,
        }),
    )
    .expect("bind private broker socket");
    let broker_shutdown = CancellationToken::new();
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
    BootContext {
        runtime_root,
        rootfs,
        sqlite_disk,
        sqlite_disk_for_assertion,
        sqlite_uuid,
        mount_root,
        repository,
        workspace,
        cgroup_root,
        rootfs_for_graceful_test,
        rootfs_for_force_test,
        expected_limits,
        timeout_provider,
        provider,
        secret_mount: Some(secret_mount),
        secret_path,
        broker_credential,
        broker_session,
        broker_shutdown: Some(broker_shutdown),
        broker_task: Some(broker_task),
    }
}

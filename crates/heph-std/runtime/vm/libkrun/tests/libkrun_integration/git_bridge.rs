use std::{
    collections::BTreeMap, env, fs, io, os::unix::net::UnixListener, path::PathBuf, thread,
    time::Duration,
};

use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, RuntimeAuthorityBootstrap, RuntimeGitBridge,
    VmEvent, VmId, VmResources, VmSpec,
};
use vm_trait::{RUNTIME_AUTHORITY_CREDENTIAL_BYTES, RUNTIME_GIT_CREDENTIAL_BYTES, VmProvider};

use crate::{ENABLE_FLAG, INTEGRATION_TEST_LOCK, support::required_path};

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

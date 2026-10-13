//! Actual configured-worker exec and supervisor death before cgroup placement.
//! No guest is started; the pidfd is test custody, never production authority.
use super::super::{
    common::{LaunchCustody, WorkerBackend, WorkerClient, WorkerProcess, WorkerSpawner},
    helpers::provider_error,
};
use crate::{LibkrunConfig, LibkrunProvider, cgroup::Cgroup};
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use vm_trait::{VmError, VmId, VmProvider, VmProviderOwnerScope};

#[path = "native/fixture.rs"]
mod fixture;
use fixture::{
    FixtureRoot, OriginalWorker, assert_placed_stopped, configuration_for_parent, placed_control,
    spawn_supervisor, wait_child, wait_kernel_death,
};

const ID: &str = "native-pre-placement";
const HELPER: &str = "provider::containment::native::supervisor_process";

fn configuration() -> LibkrunConfig {
    let runtime =
        PathBuf::from(std::env::var_os("HEPH_CONTAINMENT_RUNTIME").expect("task-owned runtime"));
    configuration_for_parent(runtime)
}

struct PausedSpawner(PathBuf);
#[async_trait::async_trait]
impl WorkerSpawner for PausedSpawner {
    async fn spawn(
        &self,
        config: Arc<LibkrunConfig>,
        spec: crate::validation::PreparedSpec,
        runtime: &std::path::Path,
        cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        let ready = self.0.clone();
        let worker = WorkerClient::launch_with_after_spawn(
            config,
            spec,
            runtime,
            cgroup,
            custody,
            move |process| stop_original(process, &ready),
        )
        .await?;
        Ok(worker)
    }
}

fn stop_original(process: &Arc<WorkerProcess>, ready: &std::path::Path) -> Result<(), VmError> {
    let pid = process
        .pid
        .and_then(|pid| Pid::from_raw(i32::try_from(pid).ok()?))
        .ok_or(VmError::InvalidState("original child PID missing"))?;
    let handle = pidfd_open(pid, PidfdFlags::empty())
        .map_err(|error| provider_error("native-test-pidfd", error))?;
    pidfd_send_signal(&handle, Signal::STOP)
        .map_err(|error| provider_error("native-test-stop", error))?;
    // The exact Child remains retained here; the fixture parent opens its pidfd
    // while this stopped child cannot be reaped or have its PID reused.
    let pending = ready.with_extension("pending");
    fs::write(&pending, pid.as_raw_nonzero().get().to_string())
        .and_then(|()| fs::rename(&pending, ready))
        .map_err(|error| provider_error("native-test-ready", error))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(VmError::InvalidState(
        "test parent did not kill original supervisor",
    ))
}

// The actual provider must retain launch custody through the spawner wait until test kill.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "subprocess helper; requires explicit native fixture environment"]
async fn supervisor_process() {
    let config = configuration();
    let scope = VmProviderOwnerScope::new(
        std::env::var("HEPH_CONTAINMENT_NAMESPACE").expect("trusted test namespace"),
        "native-containment-host".into(),
    )
    .unwrap();
    let ready = PathBuf::from(
        std::env::var_os("HEPH_CONTAINMENT_READY").expect("task-owned readiness path"),
    );
    let provider = LibkrunProvider::open_owned_with_spawner(
        config.clone(),
        &scope,
        Arc::new(PausedSpawner(ready)),
    )
    .unwrap();
    let spec = vm_trait::VmSpec {
        id: VmId(ID.into()),
        root: vm_trait::RootFilesystem::Directory {
            host_path: config.image_roots[0].clone(),
        },
        disks: vec![],
        mounts: vec![],
        guest_volumes: vec![],
        resources: vm_trait::VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        network: vm_trait::NetworkMode::Disabled,
        command: vm_trait::GuestCommand {
            program: "/bin/sleep".into(),
            args: vec!["300".into()],
            env: std::collections::BTreeMap::new(),
            working_dir: Some("/".into()),
        },
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: std::collections::BTreeMap::new(),
    };
    // The real launch publishes the record and spawns the actual binary; the
    // private hook stops precisely before add_process and the parent kills us.
    assert!(provider.provision(spec).await.is_err());
    panic!("original supervisor survived its required test kill");
}

// Retain the reopened owner through kernel-death confirmation and fixture success release.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires actual worker executable, qualified roots/KVM and dedicated cgroup-v2 delegation"]
async fn actual_supervisor_crash_before_placement_holds_then_same_scope_reopens_and_cleans() {
    assert_ne!(rustix::process::geteuid().as_raw(), 0);
    let directory = Arc::new(FixtureRoot {
        directory: Some(tempfile::tempdir().unwrap()),
        // Failed assertions preserve original evidence even if individual
        // process death is confirmed; only the full fixture clears this hold.
        held: AtomicBool::new(true),
    });
    let runtime = directory.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    // Read existing native prerequisites without mutating process-global env.
    let mut config = configuration_for_parent(runtime.clone());
    config.startup_timeout = Duration::from_secs(10);
    let scope = VmProviderOwnerScope::new(
        uuid::Uuid::new_v4().to_string(),
        "native-containment-host".into(),
    )
    .unwrap();
    let provider = LibkrunProvider::bootstrap_owned(config.clone(), &scope).unwrap();
    drop(provider);
    let ready = directory.path().join("ready");
    let mut supervisor = spawn_supervisor(&runtime, &scope, &ready, &directory);
    tokio::time::timeout(Duration::from_secs(15), async {
        while !ready.exists() {
            assert!(
                supervisor.0.try_wait().unwrap().is_none(),
                "original supervisor exited before spawn hook"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bounded actual pre-placement spawn");
    let pid = Pid::from_raw(fs::read_to_string(&ready).unwrap().parse().unwrap()).unwrap();
    let worker = OriginalWorker(
        pidfd_open(pid, PidfdFlags::empty()).unwrap(),
        Arc::clone(&directory),
    );
    let status_path = format!("/proc/{}/status", pid.as_raw_nonzero());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = fs::read_to_string(&status_path).unwrap();
            if status
                .lines()
                .any(|line| line.starts_with("State:\tT") || line.starts_with("State:\tt"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original pidfd-controlled worker actually stopped");
    let id = VmId(ID.into());
    assert!(
        fs::read_to_string(config.cgroup_root.join(ID).join("cgroup.procs"))
            .unwrap()
            .trim()
            .is_empty()
    );
    let record_path = runtime.join(super::birth::filename(&id));
    let before = fs::read(&record_path).unwrap();
    supervisor.0.kill().unwrap();
    wait_child(&mut supervisor.0).expect("bounded original supervisor reap");
    let mut placed = placed_control(&config, &directory);
    let reopened = LibkrunProvider::open_owned(config.clone(), &scope).unwrap();
    reopened.close_admission(&scope).await.unwrap();
    assert!(
        reopened
            .attempt_inventory(&scope)
            .await
            .unwrap()
            .attempts
            .is_empty()
    );
    assert!(
        reopened.cleanup_orphan_scoped(&scope, &id).await.is_err(),
        "empty new generation/cgroup cannot erase inherited unplaced custody"
    );
    assert_placed_stopped(&mut placed, &config);
    assert_eq!(fs::read(&record_path).unwrap(), before);
    assert!(runtime.join(ID).exists());
    // Resume the exact original kernel handle, never a persisted/reused PID.
    let readiness = tokio::io::unix::AsyncFd::new(worker.0.try_clone().unwrap()).unwrap();
    pidfd_send_signal(&worker.0, Signal::CONT).unwrap();
    let guard = tokio::time::timeout(Duration::from_secs(10), readiness.readable())
        .await
        .expect("normal failed-connect/IPC EOF actually exits worker")
        .unwrap();
    drop(guard);
    reopened.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    assert!(!runtime.join(ID).exists());
    assert!(!config.cgroup_root.join(ID).exists());
    assert!(!record_path.exists());
    reopened.cleanup_orphan_scoped(&scope, &id).await.unwrap();
    // pidfd readability confirms death, not a successor Child reap/DB receipt.
    wait_kernel_death(&worker.0).expect("original pidfd must positively confirm kernel death");
    directory.held.store(false, Ordering::Relaxed);
}

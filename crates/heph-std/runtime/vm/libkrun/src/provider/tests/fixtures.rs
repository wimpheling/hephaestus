use super::*;

pub(in crate::provider::tests) fn instance(
    temp: &TempDir,
    worker: Arc<MockWorker>,
) -> Arc<LibkrunInstance> {
    instance_with_wall_clock(temp, worker, Duration::from_secs(60))
}

pub(in crate::provider::tests) fn instance_with_wall_clock(
    temp: &TempDir,
    worker: Arc<MockWorker>,
    wall_clock_timeout: Duration,
) -> Arc<LibkrunInstance> {
    instance_with_options(temp, worker, wall_clock_timeout, None)
}

pub(in crate::provider::tests) fn service_instance(
    temp: &TempDir,
    worker: Arc<MockWorker>,
    timeout_duration: Duration,
    broker: Arc<ServiceBroker>,
    runtime_dir: PathBuf,
) -> Arc<LibkrunInstance> {
    service_instance_with_wall_clock(
        temp,
        worker,
        Duration::from_secs(60),
        timeout_duration,
        broker,
        runtime_dir,
    )
}

pub(in crate::provider::tests) fn service_instance_with_wall_clock(
    temp: &TempDir,
    worker: Arc<MockWorker>,
    wall_clock_timeout: Duration,
    timeout_duration: Duration,
    broker: Arc<ServiceBroker>,
    runtime_dir: PathBuf,
) -> Arc<LibkrunInstance> {
    instance_with_options(
        temp,
        worker,
        wall_clock_timeout,
        Some((timeout_duration, broker, runtime_dir)),
    )
}

pub(in crate::provider::tests) fn instance_with_options(
    temp: &TempDir,
    worker: Arc<MockWorker>,
    wall_clock_timeout: Duration,
    service: Option<(Duration, Arc<ServiceBroker>, PathBuf)>,
) -> Arc<LibkrunInstance> {
    let mut config = LibkrunConfig::new(
        temp.path(),
        vec![temp.path().to_path_buf()],
        vec![temp.path().to_path_buf()],
        vec![temp.path().to_path_buf()],
        "/bin/true",
        temp.path(),
    );
    config.startup_timeout = Duration::from_millis(50);
    config.readiness_timeout = Duration::from_millis(20);
    config.limits.wall_clock_timeout = wall_clock_timeout;
    let config = Arc::new(config);
    let provider_ids = Arc::new(ProviderInner {
        config: Arc::clone(&config),
        ids: Mutex::new(HashSet::from([VmId("test".to_owned())])),
        worker_spawner: Arc::new(ProcessWorkerSpawner),
    });
    let (events, _) = broadcast::channel(32);
    let (terminal, _) = watch::channel(None::<Terminal>);
    let (ready, _) = watch::channel(false);
    let (start_result, _) = watch::channel(None::<Result<(), ErrorSnapshot>>);
    let private_service_timeout = service.as_ref().map(|(timeout, _, _)| *timeout);
    let private_service_dispatch = service
        .as_ref()
        .map(|(_, _, _)| Arc::new(Semaphore::new(1)));
    let resources = service.map(|(_, broker, runtime_dir)| OwnedResources {
        runtime_dir,
        cgroup: Cgroup::existing(&config, "test"),
        service_broker: Some(broker),
    });
    let instance = Arc::new(LibkrunInstance {
        id: VmId("test".to_owned()),
        config,
        worker,
        state: Mutex::new(Lifecycle::Provisioned),
        terminal,
        terminal_guard: Mutex::new(()),
        ready,
        start_result,
        events,
        resources: Mutex::new(resources),
        provider_ids,
        private_http_enabled: true,
        private_service_timeout,
        private_service_dispatch,
        private_http_waiters: Mutex::new(HashMap::new()),
        next_private_http_request: AtomicU64::new(1),
    });
    instance.spawn_event_forwarder();
    instance.spawn_process_monitor();
    instance
}

pub(in crate::provider::tests) fn spec(id: &str, root: PathBuf) -> VmSpec {
    VmSpec {
        id: VmId(id.to_owned()),
        root: RootFilesystem::Directory { host_path: root },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 256,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: "/bin/true".to_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/")),
        },
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::new(),
    }
}

pub(in crate::provider::tests) fn emulated_config(
    temp: &TempDir,
) -> (LibkrunConfig, PathBuf, PathBuf, PathBuf) {
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
    let kvm = temp.path().join("kvm");
    fs::write(&kvm, "").unwrap();
    let mut config = LibkrunConfig::new(
        &runtime,
        vec![images],
        vec![disks],
        vec![mounts],
        "/bin/true",
        &cgroups,
    );
    config.kvm_device = kvm;
    config.enforce_cgroup_v2 = false;
    (config, root, runtime, cgroups)
}

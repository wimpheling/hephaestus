use super::model::*;
use super::runtime::StateSpecFactory;
use super::seed::seed_instance;
use super::*;

/// Build the real `PostgreSQL`, volume, libkrun, and orchestration fixture.
pub async fn setup() -> (
    PgPool,
    Arc<PgRunRepository>,
    PathBuf,
    PathBuf,
    UpdateScenario,
    Arc<RunOrchestrator>,
    AgentInstanceId,
) {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("Phase 1B PostgreSQL URL");
    let runtime_root = required_path("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT");
    let image_root = required_path("HEPHAESTUS_LIBKRUN_IMAGE_ROOT");
    let rootfs = required_path("HEPHAESTUS_LIBKRUN_ROOTFS");
    let disk_root = required_path("HEPHAESTUS_LIBKRUN_DISK_ROOT");
    let mount_root = required_path("HEPHAESTUS_LIBKRUN_MOUNT_ROOT");
    let cgroup_root = required_path("HEPHAESTUS_LIBKRUN_CGROUP_ROOT");
    let worker = required_path("HEPHAESTUS_LIBKRUN_WORKER");
    let volume_root = disk_root.join("phase1b-volumes");
    fs::create_dir_all(&volume_root).expect("persistent test volume root");
    let volume_root = volume_root.canonicalize().expect("canonical volume root");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect Phase 1B PostgreSQL");
    let repository = Arc::new(PgRunRepository::new(pool.clone()));
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("runtime migrations");
    let volumes = Arc::new(
        LocalVolumeStore::new(
            Arc::new(PostgresVolumeMetadataRepository::new(pool.clone())),
            LocalVolumeConfig {
                volume_root: volume_root.clone(),
                transient_runtime_roots: vec![runtime_root.clone()],
                host_id: String::from("phase1b-integration-host"),
                lease_duration: Duration::from_secs(10),
                mkfs_ext4: PathBuf::from("/usr/bin/mkfs.ext4"),
            },
        )
        .expect("volume configuration"),
    );
    volumes.initialize().await.expect("volume initialization");
    let mut provider_config = LibkrunConfig::new(
        &runtime_root,
        vec![image_root],
        vec![disk_root],
        vec![mount_root],
        worker,
        &cgroup_root,
    );
    provider_config.startup_timeout = Duration::from_secs(15);
    provider_config.readiness_timeout = Duration::from_secs(45);
    let provider = Arc::new(LibkrunProvider::new(provider_config).expect("libkrun provider"));
    let instance_id = AgentInstanceId::new();
    let scenario = seed_instance(&pool, instance_id).await;
    let factory = Arc::new(StateSpecFactory {
        rootfs,
        rollback_release: scenario.rejected.release,
        timeout_release: scenario.uncertain.release,
    });
    let repository_trait: Arc<dyn RunRepository> = repository.clone();
    let volume_trait: Arc<dyn VolumeStore> = volumes.clone();
    let provider_trait: Arc<dyn VmProvider> = provider;
    let orchestrator = Arc::new(RunOrchestrator::new(
        repository_trait,
        volume_trait,
        provider_trait,
        factory,
        128 * 1024 * 1024,
    ));

    (
        pool,
        repository,
        runtime_root,
        cgroup_root,
        scenario,
        orchestrator,
        instance_id,
    )
}

pub fn required_path(name: &str) -> PathBuf {
    env::var_os(name).map_or_else(
        || panic!("{name} must be set when {ENABLE_FLAG}=1"),
        PathBuf::from,
    )
}

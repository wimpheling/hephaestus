use super::{
    AppError, Arc, AtomicBool, BTreeMap, ControlledOciPublisher, Duration, ForgeZotOciPublisher,
    InternalRegistryTokens, LocalOciRuntime, MaterializedRoot, OciBuilderWorkerConfig,
    OciImageProductionWorker, OciWorkerError, PathBuf, PgOciImageProductionJobStore, PgPool,
    PgRegistryStore, PgRepositoryOciImagePublicationStore, RootFilesystem,
    RootfsMaterializationWorker, RwLock, SystemCommandRunner, VmOciOperation, VmOciOperationConfig,
    VmProvider, VmPublishedOciEngine, component,
};

pub struct OciBuilderWorkers {
    pub preparation: OciImageProductionWorker<
        PgOciImageProductionJobStore,
        LocalOciRuntime,
        VmPublishedOciEngine<
            PgRepositoryOciImagePublicationStore,
            InternalRegistryTokens,
            SystemCommandRunner,
        >,
    >,
    pub materialization: RootfsMaterializationWorker<PgOciImageProductionJobStore, LocalOciRuntime>,
    pub manifest: PathBuf,
    pub manifest_dirty: AtomicBool,
    pub rootfs_root: PathBuf,
    pub image_filesystems: Arc<RwLock<BTreeMap<String, RootFilesystem>>>,
    pub poll_interval: Duration,
}

impl OciBuilderWorkers {
    pub fn initialize(
        pool: PgPool,
        config: OciBuilderWorkerConfig,
        token_issuer: Arc<registry_token::RegistryTokenIssuer>,
        provider: Arc<dyn VmProvider>,
        root_images: &BTreeMap<String, RootFilesystem>,
        image_filesystems: Arc<RwLock<BTreeMap<String, RootFilesystem>>>,
    ) -> Result<Self, AppError> {
        if !config.root_manifest.is_absolute() || config.poll_interval.is_zero() {
            return Err(AppError::Configuration(String::from(
                "OCI builder manifest path must be absolute and poll interval must be positive",
            )));
        }
        let runtime = LocalOciRuntime::initialize(config.runtime)
            .map_err(component("OCI local runtime configuration"))?;
        let builder_root = root_images
            .get(config.builder_vm_image.as_str())
            .cloned()
            .ok_or_else(|| {
                AppError::Configuration(String::from("OCI builder VM root is unavailable"))
            })?;
        let verifier_root = root_images
            .get(config.verifier_vm_image.as_str())
            .cloned()
            .ok_or_else(|| {
                AppError::Configuration(String::from("OCI verifier VM root is unavailable"))
            })?;
        let operation = VmOciOperation::initialize(
            provider,
            VmOciOperationConfig {
                builder_root,
                verifier_root,
                candidate_root: runtime.output_root().to_path_buf(),
                scratch_root: config.scratch_root,
                mkfs_ext4: config.mkfs_ext4,
                verification_root: config.verification_root,
                resources: config.vm_resources,
                workload_phase_timing: config.workload_phase_timing,
            },
        )
        .map_err(component("OCI VM operation configuration"))?;
        let publication_store = PgRepositoryOciImagePublicationStore::new(
            pool.clone(),
            PgRegistryStore::new(pool.clone()),
            config.publisher.authority().clone(),
            config.publication_policy_version,
            config.publication_policy,
        );
        let publisher = ForgeZotOciPublisher::new(
            runtime.clone(),
            None,
            publication_store,
            InternalRegistryTokens {
                issuer: token_issuer,
            },
            ControlledOciPublisher::new(config.publisher, SystemCommandRunner),
        );
        let preparation = OciImageProductionWorker::new(
            PgOciImageProductionJobStore::new(pool.clone()),
            runtime.clone(),
            VmPublishedOciEngine::new(operation, publisher),
            config.preparation_worker_name,
            config.materialization_worker_name.clone(),
            config.lease,
        )
        .map_err(component("OCI preparation worker configuration"))?;
        let materialization = RootfsMaterializationWorker::new(
            PgOciImageProductionJobStore::new(pool),
            runtime,
            config.materialization_worker_name,
            config.rootfs_root.clone(),
            config.lease,
        )
        .and_then(|worker| worker.with_guest_init(config.guest_init))
        .map_err(component("OCI materialization worker configuration"))?;
        Ok(Self {
            preparation,
            materialization,
            manifest: config.root_manifest,
            manifest_dirty: AtomicBool::new(false),
            rootfs_root: config.rootfs_root,
            image_filesystems,
            poll_interval: config.poll_interval,
        })
    }

    pub async fn refresh_image_filesystems(&self) -> Result<(), OciWorkerError> {
        let roots = self.materialization.materialized_roots().await?;
        refresh_image_filesystem_cache(&roots, &self.rootfs_root, &self.image_filesystems)
    }
}

pub fn refresh_image_filesystem_cache(
    roots: &[MaterializedRoot],
    rootfs_root: &std::path::Path,
    image_filesystems: &Arc<RwLock<BTreeMap<String, RootFilesystem>>>,
) -> Result<(), OciWorkerError> {
    {
        let mut image_filesystems = image_filesystems
            .write()
            .map_err(|_| OciWorkerError::InvalidConfiguration)?;
        for root in roots {
            let canonical =
                std::fs::canonicalize(&root.root_path).map_err(OciWorkerError::Filesystem)?;
            let metadata =
                std::fs::symlink_metadata(&canonical).map_err(OciWorkerError::Filesystem)?;
            if !canonical.starts_with(rootfs_root)
                || metadata.file_type().is_symlink()
                || !metadata.is_dir()
            {
                return Err(OciWorkerError::UnsafeMaterializationPath);
            }
            image_filesystems.insert(
                root.image_reference.to_string(),
                RootFilesystem::Directory {
                    host_path: canonical,
                },
            );
        }
    }
    Ok(())
}

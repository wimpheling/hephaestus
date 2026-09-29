use crate::background_loops::build_secret_mount_manager;
use crate::connect_oci_worker;
use crate::fixture_vm::ResultFixtureProvider;
use crate::{
    AppConfig, GatewayEdgeConfig, GatewayServiceLogMaintenanceScheduler,
    LocalGatewayReleaseMaterializer, LocalKeyProvider, LocalRunRuntimeManager, LocalVolumeStore,
    LocalWorkspaceManager, OciBuilderWorkers, PgForgeRepository, PgPool, PgRunAuthorityManager,
    PostgresMailboxRepository, SecretService, VmBackendConfig,
};
use control_plane_postgres::{
    connect as connect_control_plane, connect_app as connect_application,
};
use forge_service::GitStorage;
use gateway_postgres::PostgresGatewayServiceLogStore;
use heph_run::RunSecretManager;
use heph_runtime::VmProvider;
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::ReviewControlService;
use run_postgres::PgRunRepository;
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use secret_broker::{BrokerExecutor, ServiceBrokerExecutor};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Ordered database migration expected by this application version.
pub const EXPECTED_DATABASE_MIGRATION: i64 = 100;

pub const GATEWAY_SERVICE_SERVING_CAPACITY: usize = 8;
pub const GATEWAY_SERVICE_REPLACEMENT_CAPACITY: usize = 2;
pub const GATEWAY_SERVICE_REQUEST_CAPACITY: usize = 16;
use vm_fake::FakeProvider;
use vm_libkrun::LibkrunProvider;
use volume_postgres::PostgresVolumeMetadataRepository;
use workspace_postgres::PgWorkspaceMetadataRepository;

pub struct BuildPreparation {
    pub config: AppConfig,
    pub runtime_git_socket_path: Option<PathBuf>,
    pub gateway_service_host_id: String,
    pub pool: PgPool,
    pub application_pool: PgPool,
    pub storage: Arc<GitStorage>,
    pub forge: Arc<PgForgeRepository>,
    pub run_repository: Arc<crate::PgRunRepository>,
    pub mailbox_repository: Arc<PostgresMailboxRepository>,
    pub review_repository: Arc<PostgresReviewRepository>,
    pub review_control: crate::ReviewControlService,
    pub volumes: Arc<LocalVolumeStore>,
    pub build_git_binary: PathBuf,
    pub result_artifact_root: PathBuf,
    pub workspaces: Arc<LocalWorkspaceManager>,
    pub release_artifact_root: PathBuf,
    pub run_runtime: Arc<LocalRunRuntimeManager>,
    pub gateway_release_runtime: LocalGatewayReleaseMaterializer,
    pub gateway_edge_config: Option<GatewayEdgeConfig>,
    pub gateway_secret_keys: LocalKeyProvider,
    pub gateway_handoff_root: PathBuf,
    pub gateway_handoff_key: [u8; 32],
    pub gateway_root_images: BTreeMap<String, crate::RootFilesystem>,
    pub service_log_pool: PgPool,
    pub service_log_store: Arc<PostgresGatewayServiceLogStore>,
    pub service_log_maintenance: Arc<GatewayServiceLogMaintenanceScheduler>,
    pub secret_mounts: Arc<dyn RunSecretManager>,
    pub secret_service: Arc<SecretService<LocalKeyProvider>>,
    pub runtime_git_credentials: PgRuntimeGitCredentialRepository,
    pub runtime_authority: Arc<PgRunAuthorityManager>,
    pub secret_broker_executor: Arc<dyn crate::BrokerExecutor>,
    pub provider: Arc<dyn VmProvider>,
    pub image_filesystems: Arc<RwLock<BTreeMap<String, crate::RootFilesystem>>>,
    pub oci_builder_workers: Option<Arc<OciBuilderWorkers>>,
}

// Keep ordered startup setup in one function so dependency acquisition and error mapping remain auditable.
#[allow(clippy::too_many_lines)]
pub async fn prepare(mut config: AppConfig) -> Result<BuildPreparation, AppError> {
    let runtime_git_socket_path = match &mut config.vm_backend {
        VmBackendConfig::Libkrun(provider) => {
            let path = provider
                .runtime_git_socket_path
                .get_or_insert_with(|| provider.runtime_root.join("runtime-git.sock"))
                .clone();
            Some(path)
        }
        VmBackendConfig::CustomWithRuntimeGitSocket {
            runtime_git_socket_path,
            ..
        } => Some(runtime_git_socket_path.clone()),
        VmBackendConfig::Fake | VmBackendConfig::FixtureResult | VmBackendConfig::Custom(_) => None,
    };
    config.validate()?;
    let gateway_service_host_id = config.volumes.host_id.clone();
    if let VmBackendConfig::Libkrun(provider) = &mut config.vm_backend {
        if provider
            .broker_socket_path
            .as_ref()
            .is_some_and(|path| path != &config.secret_broker_socket)
        {
            return Err(AppError::Configuration(String::from(
                "libkrun broker socket does not match the application broker",
            )));
        }
        provider.broker_socket_path = Some(config.secret_broker_socket.clone());
    }
    let normalized_config = config.clone();
    let pool = connect_control_plane(&config.database_url, 20)
        .await
        .map_err(component("PostgreSQL connection"))?;
    verify_database_contract(&pool).await?;
    let application_pool = connect_application(&config.database_url, 10)
        .await
        .map_err(component("PostgreSQL application-role connection"))?;

    let storage = Arc::new(
        GitStorage::initialize(&config.repository_root)
            .await
            .map_err(component("Git storage"))?,
    );
    let forge = Arc::new(
        PgForgeRepository::new(pool.clone(), Arc::clone(&storage))
            .with_authorizer(Arc::new(authz_postgres::PostgresMelangeAuthorizer)),
    );
    let run_repository = Arc::new(PgRunRepository::new(pool.clone()));
    let mailbox_repository = Arc::new(PostgresMailboxRepository::new(pool.clone()));
    let review_repository = Arc::new(PostgresReviewRepository::new(pool.clone()));
    let review_locator = Arc::new(GitRepositoryLocator::new(Arc::clone(&storage)));
    let review_control = ReviewControlService::new(
        Arc::clone(&review_repository) as Arc<dyn review_service::ReviewRepository>,
        review_locator,
    );
    let volume_metadata = Arc::new(PostgresVolumeMetadataRepository::new(pool.clone()));
    let volumes = Arc::new(
        LocalVolumeStore::new(volume_metadata, config.volumes)
            .map_err(component("volume configuration"))?,
    );
    volumes
        .initialize()
        .await
        .map_err(component("volume initialization"))?;
    let build_git_binary = config.workspaces.git_binary.clone();
    let result_artifact_root = config.workspaces.artifact_root.clone();
    let workspace_repository = Arc::new(PgWorkspaceMetadataRepository::new(pool.clone()));
    let mut workspaces = LocalWorkspaceManager::new(
        Arc::clone(&workspace_repository) as Arc<dyn heph_runtime::WorkspaceMetadataRepository>,
        Arc::clone(&workspace_repository) as Arc<dyn heph_runtime::ResultRepository>,
        config.workspaces,
    )
    .map_err(component("workspace configuration"))?;
    workspaces
        .initialize()
        .map_err(component("workspace initialization"))?;
    let result_artifact_root =
        std::fs::canonicalize(result_artifact_root).map_err(component("result artifact root"))?;
    let workspaces = Arc::new(workspaces);
    let release_artifact_root = config.run_runtime.release_artifact_root.clone();
    let run_runtime = Arc::new(
        LocalRunRuntimeManager::initialize(run_repository.clone(), config.run_runtime)
            .map_err(component("run runtime initialization"))?,
    );
    let gateway_release_runtime = LocalGatewayReleaseMaterializer {
        runtime: run_runtime.gateway_release_runtime(),
    };
    let gateway_edge_config = config.gateway_edge.take();
    let gateway_secret_keys = config.secret_keys.clone();
    let gateway_handoff_root = config.runtime_authority_handoff_root.clone();
    let gateway_handoff_key = config.runtime_authority_handoff_key;
    let gateway_root_images = config.root_images.clone();
    // Service-log append and retention use one dedicated worker pool. It
    // is created even when the optional gateway edge is disabled so the
    // retention scheduler has stable ownership and shutdown semantics.
    let service_log_pool = connect_oci_worker(&config.database_url, 2)
        .await
        .map_err(component("service-log PostgreSQL connection"))?;
    let service_log_store = Arc::new(PostgresGatewayServiceLogStore::new(
        service_log_pool.clone(),
    ));
    let service_log_projects: Arc<dyn gateway_edge::GatewayServiceLogMaintenanceProjects> =
        service_log_store.clone();
    let service_log_maintenance_port: Arc<dyn gateway_edge::GatewayServiceLogMaintenance> =
        service_log_store.clone();
    let service_log_maintenance = Arc::new(GatewayServiceLogMaintenanceScheduler::new(
        service_log_projects,
        service_log_maintenance_port,
        gateway_edge::GatewayServiceLogMaintenancePolicy::default(),
    ));
    let (secret_mounts, secret_runtime, secret_service) = build_secret_mount_manager(
        pool.clone(),
        &config.database_url,
        config.secret_keys,
        config.secret_mounts,
    )
    .await?;
    let runtime_git_credentials = PgRuntimeGitCredentialRepository::new(pool.clone());
    let runtime_authority = Arc::new(PgRunAuthorityManager::new(
        pool.clone(),
        runtime_git_credentials.clone(),
        config.runtime_authority_handoff_root,
        config.runtime_authority_handoff_key,
        config.runtime_authority_session_ttl,
    )?);
    let secret_broker_executor: Arc<dyn BrokerExecutor> = Arc::new(ServiceBrokerExecutor::new(
        secret_runtime,
        config.secret_broker_adapter,
    ));

    let provider: Arc<dyn VmProvider> = match config.vm_backend {
        VmBackendConfig::Fake => Arc::new(FakeProvider::new()),
        VmBackendConfig::FixtureResult => Arc::new(ResultFixtureProvider),
        VmBackendConfig::Custom(provider)
        | VmBackendConfig::CustomWithRuntimeGitSocket { provider, .. } => provider,
        VmBackendConfig::Libkrun(provider) => {
            Arc::new(LibkrunProvider::new(*provider).map_err(component("libkrun provider"))?)
        }
    };
    let image_filesystems = Arc::new(RwLock::new(config.root_images.clone()));
    let oci_builder_workers = match config.oci_builder.take() {
        Some(worker) => {
            let worker_pool = connect_oci_worker(&config.database_url, 4)
                .await
                .map_err(component("OCI worker PostgreSQL connection"))?;
            Some(Arc::new(OciBuilderWorkers::initialize(
                worker_pool,
                worker,
                Arc::clone(&config.registry.token_issuer),
                Arc::clone(&provider),
                &config.root_images,
                Arc::clone(&image_filesystems),
            )?))
        }
        None => None,
    };
    Ok(BuildPreparation {
        config: normalized_config,
        runtime_git_socket_path,
        gateway_service_host_id,
        pool,
        application_pool,
        storage,
        forge,
        run_repository,
        mailbox_repository,
        review_repository,
        review_control,
        volumes,
        build_git_binary,
        result_artifact_root,
        workspaces,
        release_artifact_root,
        run_runtime,
        gateway_release_runtime,
        gateway_edge_config,
        gateway_secret_keys,
        gateway_handoff_root,
        gateway_handoff_key,
        gateway_root_images,
        service_log_pool,
        service_log_store,
        service_log_maintenance,
        secret_mounts,
        secret_service,
        runtime_git_credentials,
        runtime_authority,
        secret_broker_executor,
        provider,
        image_filesystems,
        oci_builder_workers,
    })
}

async fn verify_database_contract(pool: &PgPool) -> Result<(), AppError> {
    let (migration, melange) = control_plane_postgres::verify_contract(pool)
        .await
        .map_err(component("database contract check"))?;
    if migration != Some(crate::EXPECTED_DATABASE_MIGRATION) {
        return Err(AppError::Configuration(format!(
            "database migration is {migration:?}; expected {}",
            crate::EXPECTED_DATABASE_MIGRATION
        )));
    }
    if !melange {
        return Err(AppError::Configuration(String::from(
            "Mélange check_permission dispatcher is missing",
        )));
    }
    Ok(())
}

pub fn component<Error: std::fmt::Display>(
    name: &'static str,
) -> impl FnOnce(Error) -> AppError + Copy {
    move |error| AppError::Component {
        component: name,
        message: error.to_string(),
    }
}

/// Application construction, startup, supervision, or shutdown failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    /// Static or migration configuration is invalid.
    #[error("invalid application configuration: {0}")]
    Configuration(String),
    /// One application component failed.
    #[error("{component} failed: {message}")]
    Component {
        /// Component name.
        component: &'static str,
        /// Non-sensitive failure.
        message: String,
    },
    /// Readiness barrier failed.
    #[error("application readiness failed: {0}")]
    Readiness(String),
    /// A supervised task failed.
    #[error("supervised application task failed: {0}")]
    Task(String),
    /// An operation timed out.
    #[error("application operation timed out: {0}")]
    Timeout(String),
    /// Resource shutdown failed.
    #[error("resource shutdown failed: {0}")]
    Shutdown(String),
}

#[path = "build_preparation/error.rs"]
mod error;
pub use error::{AppError, component};

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
    connect as connect_control_plane, connect_app as connect_application, connect_worker,
};
use forge_storage::GitStorage;
use gateway_postgres::PostgresGatewayServiceLogStore;
use heph_run::RunSecretManager;
use heph_runtime::VmProvider;
use review_git::GitReviewPublisher;
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::ReviewControlService;
use run_domain::FreshRunExecutionProfile;
use run_postgres::PgRunRepository;
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use secret_broker::{BrokerExecutor, ServiceBrokerExecutor};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Ordered database migration expected by this application version.
pub const EXPECTED_DATABASE_MIGRATION: i64 = 143;

pub const GATEWAY_SERVICE_SERVING_CAPACITY: usize = 8;
pub const GATEWAY_SERVICE_REPLACEMENT_CAPACITY: usize = 2;
pub const GATEWAY_SERVICE_REQUEST_CAPACITY: usize = 16;
use vm_fake::FakeProvider;
use vm_libkrun::LibkrunProvider;
use volume_postgres::{PostgresVolumeMetadataRepository, ScalarMetadataSchema};
use workspace_postgres::PgWorkspaceMetadataRepository;

pub struct BuildPreparation {
    pub config: AppConfig,
    pub managed_backend: Option<crate::managed_backend::NodeLease>,
    pub runtime_git_socket_path: Option<PathBuf>,
    pub gateway_service_host_id: String,
    pub pool: PgPool,
    pub application_pool: PgPool,
    pub worker_pool: PgPool,
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
pub async fn prepare(
    mut config: AppConfig,
    service: Option<&crate::ManagedBackendSupervisor>,
) -> Result<BuildPreparation, AppError> {
    if config.managed_backend.is_some() && service.is_none() {
        return Err(AppError::Configuration(
            "managed configuration requires an explicit process-owned supervisor".into(),
        ));
    }
    let mut runtime_git_socket_path = match &mut config.vm_backend {
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
    if let Some(managed) = &config.managed_backend {
        runtime_git_socket_path = Some(managed.runtime_git_socket.clone());
    }
    let normalized_config = config.clone();
    let pool = connect_control_plane(&config.database_url, 20)
        .await
        .map_err(component("PostgreSQL connection"))?;
    verify_database_contract(&pool).await?;
    let application_pool = connect_application(&config.database_url, 10)
        .await
        .map_err(component("PostgreSQL application-role connection"))?;

    let worker_pool = connect_worker(&config.database_url, 8)
        .await
        .map_err(component("PostgreSQL run-worker connection"))?;
    // Every ordinary or managed build verifies the complete public143 contract
    // on its actual three pools before ANY filesystem initializer.
    control_plane_postgres::verify_qualified_invocation_contract(
        &pool,
        &worker_pool,
        &application_pool,
    )
    .await
    .map_err(component("public143 three-pool contract"))?;
    let managed_backend = crate::managed_backend::prepare(
        config.clone(),
        &pool,
        &worker_pool,
        &application_pool,
        service,
    )
    .await?;

    let storage = Arc::new(
        GitStorage::initialize(&config.repository_root)
            .await
            .map_err(component("Git storage"))?,
    );
    let forge = Arc::new(
        PgForgeRepository::new(pool.clone(), Arc::clone(&storage))
            .with_authorizer(Arc::new(authz_postgres::PostgresMelangeAuthorizer)),
    );
    // This explicit choice applies only to fresh ordinary Runs. Historical
    // originals remain governed by their persisted profile; no owner is adopted.
    let run_repository = Arc::new(
        PgRunRepository::new(worker_pool.clone())
            .with_execution_profile(FreshRunExecutionProfile::LegacyScalar),
    );
    let mailbox_repository = Arc::new(
        PostgresMailboxRepository::new(worker_pool.clone())
            .with_execution_profile(FreshRunExecutionProfile::LegacyScalar),
    );
    let review_repository = Arc::new(PostgresReviewRepository::new(pool.clone()));
    let review_locator = Arc::new(GitRepositoryLocator::new(Arc::clone(&storage)));
    let review_control = ReviewControlService::with_git(
        Arc::clone(&review_repository) as Arc<dyn review_service::ReviewRepository>,
        Arc::new(GitReviewPublisher::new(review_locator)),
    );
    let volume_metadata = Arc::new(
        PostgresVolumeMetadataRepository::new(worker_pool.clone())
            .with_scalar_schema(ScalarMetadataSchema::Profiled),
    );
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
    let workspace_repository = Arc::new(PgWorkspaceMetadataRepository::new(worker_pool.clone()));
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
        application_pool.clone(),
        worker_pool.clone(),
        config.secret_keys,
        config.secret_mounts,
    )
    .await?;
    let runtime_git_credentials = PgRuntimeGitCredentialRepository::new(worker_pool.clone());
    let runtime_authority = Arc::new(PgRunAuthorityManager::new(
        worker_pool.clone(),
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
        managed_backend,
        runtime_git_socket_path,
        gateway_service_host_id,
        pool,
        application_pool,
        worker_pool,
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

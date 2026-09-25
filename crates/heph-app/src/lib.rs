//! Production composition root and supervised daemon lifecycle.

mod application;
mod event_adapter;
mod event_cursor;
pub mod rpc;
mod runtime_git_listener;
mod service_log_maintenance;
pub(crate) mod ui_audit;
mod ui_bootstrap;
mod ui_browser_content;
mod ui_context;
mod ui_origin_config;
mod ui_origin_wiring;
mod ui_repository_git;

pub use ui_origin_config::{UiOriginConfig, UiOriginConfigError};

#[path = "config_types.rs"]
mod config_types;
pub use config_types::{
    AppConfig, GatewayEdgeConfig, OciBuilderWorkerConfig, OidcConfig, RegistryConfig,
    RuntimePolicy, VmBackendConfig,
};

#[path = "config_validation.rs"]
mod config_validation;

#[path = "app_runtime.rs"]
mod app_runtime;
#[path = "registry_adapters.rs"]
mod registry_adapters;
use registry_adapters::{
    InternalRegistryTokens, PostgresRegistryNotificationInbox, PostgresRegistryReconciliation,
    PostgresRegistryScopeAuthorizer, registry_caller_authentication, registry_reconciliation_loop,
};

#[path = "oci_workers.rs"]
mod oci_workers;
use oci_workers::OciBuilderWorkers;

use app_runtime::{
    GatewayEdgeRuntime, LocalGatewayReleaseMaterializer, ProviderGatewayRuntimeLauncher,
};

#[path = "gateway_dispatch.rs"]
mod gateway_dispatch;
use gateway_dispatch::{PrivateGatewayDispatcherState, gateway_limits, private_gateway_dispatch};

#[path = "gateway_service_state.rs"]
mod gateway_service_state;
use gateway_service_state::clone_service_supervisor_context;

#[path = "gateway_service_cleanup.rs"]
mod gateway_service_cleanup;

#[path = "gateway_reconciliation_loop.rs"]
mod gateway_reconciliation_loop;

#[path = "gateway_reconciliation.rs"]
mod gateway_reconciliation;
#[cfg(test)]
use gateway_reconciliation::gateway_reconciliation_loop;
use gateway_reconciliation::gateway_reconciliation_loop_with_context;
#[cfg(test)]
use gateway_reconciliation_loop::gateway_reconciliation_loop_with_boot;

#[path = "background_loops.rs"]
mod background_loops;
#[cfg(test)]
use background_loops::write_oci_manifest_if_dirty;
use background_loops::{
    OutboxWorker, build_secret_mount_manager, mailbox_recovery_loop, oci_builder_loop,
    reap_failed_start, secret_revocation_loop, spawn_runtime_git_listener,
};

#[path = "update_completion.rs"]
mod update_completion;
#[cfg(test)]
use update_completion::deterministic_update_hook_run_id;
use update_completion::{UpdateRunCompletion, update_admission_reconciliation_loop};

#[path = "command_transport.rs"]
mod command_transport;
#[cfg(test)]
use command_transport::build_delivery_requires_redelivery;

#[path = "outbox_flush.rs"]
mod outbox_flush;
use outbox_flush::{FlushDiagnostics, FlushPublisher, flush_publisher, flush_until_quiescent};

#[path = "runtime_authority_manager.rs"]
mod runtime_authority_manager;
use runtime_authority_manager::PgRunAuthorityManager;
pub use runtime_authority_manager::RunEventKind;

#[path = "vm_spec_factory.rs"]
mod vm_spec_factory;
use vm_spec_factory::PgAgentVmSpecFactory;
#[cfg(test)]
use vm_spec_factory::StoredNetworkAccess;
#[cfg(test)]
use vm_spec_factory::{guest_environment, validate_runtime_policy};

#[path = "fixture_vm.rs"]
mod fixture_vm;
#[cfg(test)]
mod tests;
mod ui_listener;
use command_transport::{build_loop, command_loop, mailbox_command_loop};
use fixture_vm::ResultFixtureProvider;

/// Test-only lifecycle synchronization hooks used by daemon integration tests.
#[cfg(feature = "test-fixtures")]
#[doc(hidden)]
pub mod test_hooks {
    pub use crate::application::commands::{
        CreateUpdateAdmissionBarrier, CreateUpdateAdmissionBarrierGuard,
        install_create_update_admission_barrier,
    };
}

use async_trait::async_trait;
use axum::{
    Router,
    routing::{any, get},
};
use build_orchestrator::{BuildExecutionError, BuildExecutor, BuildExecutorConfig};
use build_postgres::PgBuildRepository;
use builder_catalog_domain::OciImageReference;
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use control_plane_postgres::{
    ControlPlanePool, connect as connect_control_plane, connect_app as connect_application,
    connect_worker as connect_oci_worker,
};
use event_postgres::{ReleaseOutboxPublisher, ensure_release_jetstream_topology};
use forge_postgres::PgForgeRepository;
use forge_service::{
    ForgeNatsOutboxPublisher, GitStorage, ensure_build_consumer, ensure_forge_jetstream_topology,
};
use gateway_edge::{
    GatewayDispatcher, GatewayInboundSecretResolver, GatewayProvider, GatewayRequestDispatcher,
    GatewayRuntimeLauncher, GatewayRuntimeService, GatewayServiceArtifact,
    GatewayServiceArtifactKind, GatewayServiceBootRecovery, GatewayServiceBootRecoveryContext,
    GatewayServiceClaimResolutionStore, GatewayServiceCleanupDriverPolicy,
    GatewayServiceExpiredClaimRecovery, GatewayServiceHandler, GatewayServiceIdentity,
    GatewayServiceLogStore, GatewayServiceLogWriterConfig, GatewayServiceMaterializer,
    GatewayServiceOwner, GatewayServiceRegistry, GatewayServiceSupervisor,
    GatewayServiceSupervisorContext, GatewayServiceSupervisorPolicy, LocalCaddyAdministration,
    LocalCaddyConfigurationTemplate, LocalCaddyGatewayProvider, PrivateHttpVmGatewayHandler,
    ServiceLogWriterPolicy,
};
use gateway_postgres::{
    GatewayReleaseArtifact, GatewayReleaseArtifactKind, GatewayReleaseMaterializer,
    PostgresGatewayEdgeAuthority, PostgresGatewayExecutionTargetResolver,
    PostgresGatewayMailboxPublisher, PostgresGatewayReleaseResolver,
    PostgresGatewayServiceFailureStore, PostgresGatewayServiceLaunchResolver,
    PostgresGatewayServiceLogStore, PostgresGatewayServiceOwnership, PostgresGatewayServiceTargets,
};
use git_http::{
    CompositeGitAuthenticator, GitAuthenticator, GitHttpLimits, GitHttpService,
    OidcGitAuthenticator, PostgresGitAuthorizer, RuntimeGitHttpAuthenticator,
};
use heph_run::{
    CancelRun, CompositeRunCompletionObserver, Run, RunCompletionError, RunCompletionObserver,
    RunKind, RunOrchestrator, RunRepository, RunRuntimeArtifact, RunRuntimeArtifactKind,
};
use heph_secret::EphemeralSecretConfig;
use identity_application::{BrowserSessionStore, IdempotentIdentityResolver};
use identity_oidc::OidcVerifier;
use identity_postgres::{PostgresBrowserSessionStore, PostgresIdentityStore};
use jsonwebtoken::{Algorithm, DecodingKey};
use mailbox_dispatch::{
    MailboxCommandHandler, MailboxDispatchStore, MailboxOutboxPublisher, MailboxRunCompletion,
    MailboxRunResources, NatsMailboxCommandHandler, ensure_mailbox_jetstream_topology,
};
use mailbox_postgres::PostgresMailboxRepository;
use oci_builder_postgres::{PgOciImageProductionJobStore, PgRepositoryOciImagePublicationStore};
use oci_builder_runtime_local::{
    ForgeZotOciPublisher, LocalOciRuntime, LocalOciRuntimeConfig, VmOciOperation,
    VmOciOperationConfig, VmPublishedOciEngine,
};
use oci_builder_worker::{
    MaterializedRoot, OciImageProductionWorker, OciWorkerError, RegistryPublisherTokenIssuer,
    RootfsMaterializationWorker,
};
use registry_domain::{PolicyVersion, RegistryNamespace, SupplyChainPolicy};
use registry_http::{RegistryAuthorizationError, RegistryTokenHttpService};
use registry_notification::{NotificationAction, NotificationObservation};
use registry_notification_http::{
    InboxDisposition, RegistryInboxError, RegistryNotificationHttpService,
    RegistryNotificationInbox,
};
use registry_postgres::{
    NewRegistryNotification, NotificationCompletion as PgNotificationCompletion, PgRegistryStore,
    RegistryNotificationAction, RegistryNotificationTarget,
};
use registry_publisher::{ControlledOciPublisher, PublisherConfiguration, SystemCommandRunner};
use registry_reconciler::{
    ClaimedNotification, NotificationCompletion, NotificationInbox, PublicationIntents,
    ReconciliationAction, ReconciliationActionExecutor, ReconciliationPortError,
    RegistryReconciler,
};
use registry_token::{
    AuthorizationDecision as RegistryAuthorizationDecision, RegistryAction, RepositoryActions,
    RepositoryName, ScopeRequest, TokenSubject, UnixTimestamp,
};
use registry_zot::{RegistryPullTokenProvider, ZotClientConfig, ZotClientError, ZotHttpRegistry};
use release_artifact_store::LocalArtifactStore;
use release_domain::BuildRequestId;
use release_postgres::{
    PgUiBrowserServingStore, PgUiBrowserSessionStore, PgUiGenerationHostResolver,
    PgUiRequestAuditRepository, ReleaseService, ReleaseServiceError,
};
use release_service::{
    UiBrowserRepositoryGitAuthorization, UiBrowserSessionStore, UiGenerationHostResolver,
};
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::{NatsControlHandler, ReviewControlService, ReviewOutboxPublisher};
use run_orchestrator::{NatsCommandHandler, ensure_jetstream_topology};
use run_postgres::PgRunRepository;
use run_runtime_local::{
    GatewayServiceIdentity as LocalGatewayServiceIdentity, LocalGatewayReleaseRuntime,
    LocalRunRuntimeConfig, LocalRunRuntimeManager,
};
use runtime_authority::{GatewayRuntimeAuthorityIssuer, RuntimeHandoffStore};
use runtime_authority_postgres::PgGatewayRuntimeAuthorityIssuer;
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use runtime_handoff_local::EncryptedFileHandoffStore;
use runtime_types::{CommandId, RunId};
use secret_application::BrokerAdapter;
use secret_broker::{BrokerExecutor, BrokerServer, ServiceBrokerExecutor};
use secret_postgres::{GatewayIngressSecretResolver, SecretService};
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde::Deserialize;
use service_log_maintenance::GatewayServiceLogMaintenanceScheduler;
type PgPool = ControlPlanePool;
use heph_runtime::{
    GuestCommand, NetworkMode, RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId,
    VmInstance, VmMetric, VmMount, VmProvider, VmResources, VmSpec,
};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::OffsetDateTime;
use tokio::{
    sync::{Mutex, Semaphore, oneshot},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vm_fake::FakeProvider;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_postgres::PostgresVolumeMetadataRepository;
use workspace_local::{LocalWorkspaceConfig, LocalWorkspaceManager};
use workspace_postgres::PgWorkspaceMetadataRepository;

/// Ordered database migration expected by this application version.
pub const EXPECTED_DATABASE_MIGRATION: i64 = 100;

const GATEWAY_SERVICE_SERVING_CAPACITY: usize = 8;
const GATEWAY_SERVICE_REPLACEMENT_CAPACITY: usize = 2;
const GATEWAY_SERVICE_REQUEST_CAPACITY: usize = 16;

/// Constructed application whose external tasks have not started.
pub struct HephaestusApp {
    pool: PgPool,
    application_pool: PgPool,
    service_log_pool: PgPool,
    nats_client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
    forge: Arc<PgForgeRepository>,
    storage: Arc<GitStorage>,
    identity_store: Arc<PostgresIdentityStore>,
    git_authenticator: Arc<dyn GitAuthenticator>,
    git_authorizer: Arc<PostgresGitAuthorizer>,
    git_backend: PathBuf,
    git_pre_receive_hook: PathBuf,
    git_limits: GitHttpLimits,
    registry: RegistryConfig,
    http_listen: SocketAddr,
    run_repository: Arc<PgRunRepository>,
    mailbox_repository: Arc<PostgresMailboxRepository>,
    review_repository: Arc<PostgresReviewRepository>,
    review_control: ReviewControlService,
    orchestrator: Arc<RunOrchestrator>,
    build_executor: Arc<BuildExecutor>,
    oci_builder_workers: Option<Arc<OciBuilderWorkers>>,
    artifact_store: LocalArtifactStore,
    result_artifact_root: PathBuf,
    release_service: Arc<ReleaseService>,
    update_completion: Arc<UpdateRunCompletion>,
    secret_service: Arc<SecretService<LocalKeyProvider>>,
    rpc_mediator_signing_key: [u8; 32],
    internal_platform_policy: release_domain::RuntimePolicy,
    internal_platform_policy_version: String,
    secret_broker_socket: PathBuf,
    secret_broker_executor: Arc<dyn BrokerExecutor>,
    service_log_maintenance: Arc<GatewayServiceLogMaintenanceScheduler>,
    gateway_edge: Option<GatewayEdgeRuntime>,
    worker_concurrency: usize,
    outbox_poll_interval: Duration,
    outbox_batch_size: i64,
    startup_timeout: Duration,
    shutdown_timeout: Duration,
    runtime_git_socket_path: Option<PathBuf>,
}

impl HephaestusApp {
    /// Validates configuration and constructs every production dependency.
    ///
    /// This does not bind listeners or spawn background tasks.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid configuration, incompatible migrations, or
    /// an unavailable required dependency.
    // Keeping dependency construction in the composition root makes the
    // production security boundaries directly auditable.
    #[allow(clippy::too_many_lines)]
    pub async fn build(mut config: AppConfig) -> Result<Self, AppError> {
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
            VmBackendConfig::Fake | VmBackendConfig::FixtureResult | VmBackendConfig::Custom(_) => {
                None
            }
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
        let result_artifact_root = std::fs::canonicalize(result_artifact_root)
            .map_err(component("result artifact root"))?;
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
        let gateway_edge = if let Some(gateway) = gateway_edge_config {
            // Gateway runtime snapshots and sessions are worker-owned
            // immutable authority records. Keep issuance on a dedicated
            // worker-role pool rather than leaking those writes through the
            // user-scoped control-plane pool.
            let gateway_authority_pool = connect_oci_worker(&config.database_url, 4)
                .await
                .map_err(component("gateway runtime authority PostgreSQL connection"))?;
            let service_owner = GatewayServiceOwner::new(gateway_service_host_id, Uuid::new_v4())
                .map_err(component("gateway service owner"))?;
            let service_registry = GatewayServiceRegistry::new(
                GATEWAY_SERVICE_SERVING_CAPACITY + GATEWAY_SERVICE_REPLACEMENT_CAPACITY,
                GATEWAY_SERVICE_REQUEST_CAPACITY,
            )
            .map_err(component("gateway service registry"))?;
            let issuer_handoff =
                EncryptedFileHandoffStore::new(gateway_handoff_root.clone(), gateway_handoff_key)
                    .map_err(component("gateway runtime authority handoff"))?;
            let issuer: Arc<dyn GatewayRuntimeAuthorityIssuer> =
                Arc::new(PgGatewayRuntimeAuthorityIssuer::new(
                    gateway_authority_pool.clone(),
                    issuer_handoff,
                    authz_postgres::AUTHORIZATION_MODEL_VERSION,
                ));
            let authority = PostgresGatewayEdgeAuthority::new(pool.clone(), gateway_limits())
                .with_runtime_authority(Arc::clone(&issuer), Duration::from_secs(30))
                .map_err(component("gateway runtime authority"))?;
            let recovery_authority =
                PostgresGatewayEdgeAuthority::new(gateway_authority_pool.clone(), gateway_limits());
            let ui_authority =
                PostgresGatewayEdgeAuthority::new(gateway_authority_pool.clone(), gateway_limits())
                    .with_runtime_authority(Arc::clone(&issuer), Duration::from_secs(30))
                    .map_err(component("UI gateway runtime authority"))?;
            let gateway_release_materializer = Arc::new(gateway_release_runtime);
            let gateway_release_materializer_port: Arc<dyn GatewayReleaseMaterializer> =
                gateway_release_materializer.clone();
            let gateway_service_materializer: Arc<dyn GatewayServiceMaterializer> =
                gateway_release_materializer.clone();
            let service_ownership = Arc::new(PostgresGatewayServiceOwnership::new(
                gateway_authority_pool.clone(),
            ));
            let service_claim_resolution: Arc<dyn GatewayServiceClaimResolutionStore> =
                service_ownership.clone();
            let service_expired_claim_recovery: Arc<dyn GatewayServiceExpiredClaimRecovery> =
                service_ownership.clone();
            let service_failure_store = Arc::new(PostgresGatewayServiceFailureStore::new(
                gateway_authority_pool.clone(),
            ));
            let service_log_writer = GatewayServiceLogWriterConfig::new(
                service_log_store.clone() as Arc<dyn GatewayServiceLogStore>,
                ServiceLogWriterPolicy::default(),
            );
            let service_launch_resolver = Arc::new(
                PostgresGatewayServiceLaunchResolver::new(
                    gateway_authority_pool.clone(),
                    gateway_root_images.clone(),
                )
                .with_service_materializer(Arc::clone(&gateway_service_materializer)),
            );
            let service_targets = Arc::new(PostgresGatewayServiceTargets::new(
                gateway_authority_pool.clone(),
            ));
            let resolver_handoff: Arc<dyn RuntimeHandoffStore> = Arc::new(
                EncryptedFileHandoffStore::new(gateway_handoff_root, gateway_handoff_key)
                    .map_err(component("gateway runtime resolver handoff"))?,
            );
            let releases = PostgresGatewayReleaseResolver::new(
                pool.clone(),
                gateway_root_images.clone(),
                resolver_handoff,
            )
            .with_release_materializer(gateway_release_materializer_port);
            let runtime = GatewayRuntimeService::new(
                releases,
                ProviderGatewayRuntimeLauncher {
                    provider: Arc::clone(&provider),
                },
            );
            let stateless_handler = PrivateHttpVmGatewayHandler::new(runtime);
            let service_supervisor_context = Arc::new(GatewayServiceSupervisorContext {
                owner: service_owner.clone(),
                policy: GatewayServiceSupervisorPolicy::default(),
                ownership: service_ownership.clone(),
                failure_store: service_failure_store.clone(),
                resolver: service_launch_resolver.clone(),
                provider: Arc::clone(&provider),
                targets: service_targets.clone(),
                registry: service_registry.clone(),
                service_authority: gateway.public_authority.clone(),
            });
            let service_policy = service_supervisor_context.policy;
            let service_boot_context = GatewayServiceBootRecoveryContext {
                owner: service_supervisor_context.owner.clone(),
                cleanup_policy: GatewayServiceCleanupDriverPolicy {
                    lease: service_policy.lease,
                    database_timeout: service_policy.instance.probe_timeout,
                },
                shutdown_timeout: service_policy.instance.shutdown_timeout,
                ownership: service_ownership.clone(),
                exact_recovery: service_ownership.clone(),
                targets: service_targets.clone(),
                failure_store: service_failure_store.clone(),
                resolver: service_launch_resolver.clone(),
                provider: Arc::clone(&service_supervisor_context.provider),
            };
            GatewayServiceSupervisor::new(clone_service_supervisor_context(
                &service_supervisor_context,
            ))
            .map_err(component("gateway service supervisor"))?;
            let handler = Arc::new(
                GatewayServiceHandler::new(
                    PostgresGatewayExecutionTargetResolver::new(gateway_authority_pool.clone()),
                    stateless_handler,
                    service_registry,
                    service_owner,
                )
                .map_err(component("gateway service handler"))?,
            );
            let ingress_pool = connect_control_plane(&config.database_url, 4)
                .await
                .map_err(component("gateway secret resolver PostgreSQL connection"))?;
            let inbound: Arc<dyn GatewayInboundSecretResolver> =
                Arc::new(GatewayIngressSecretResolver::new(
                    ingress_pool,
                    EncryptedStore::new(gateway_secret_keys),
                ));
            let mailbox: Arc<dyn gateway_edge::GatewayMailboxPublisher> =
                Arc::new(PostgresGatewayMailboxPublisher::new(pool.clone()));
            let dispatcher: Arc<dyn GatewayRequestDispatcher> = Arc::new(
                GatewayDispatcher::new(authority.clone(), Arc::clone(&handler), authority.clone())
                    .with_inbound_secret_resolver(Arc::clone(&inbound))
                    .with_mailbox_publisher(Arc::clone(&mailbox)),
            );
            let ui_dispatcher = gateway.ui_origin.as_ref().map(|ui| {
                let ui_core = Arc::new(
                    GatewayDispatcher::new(
                        ui_authority.clone(),
                        Arc::clone(&handler),
                        ui_authority.clone(),
                    )
                    .with_inbound_secret_resolver(Arc::clone(&inbound))
                    .with_mailbox_publisher(Arc::clone(&mailbox)),
                );
                Arc::new(ui_origin_wiring::RealUiGatewayDispatcher::new(
                    ui_core,
                    Arc::new(ui_authority.clone()),
                    ui.namespace().clone(),
                    ui.public_port(),
                )) as Arc<dyn ui_browser_content::UiGatewayDispatcher>
            });
            let administration = LocalCaddyAdministration::new(&gateway.caddy_admin_url)
                .map_err(component("gateway Caddy administration"))?;
            let template = LocalCaddyConfigurationTemplate::new(
                &gateway.caddy_configuration_template,
                gateway.caddy_server_name,
            )
            .map_err(component("gateway Caddy configuration template"))?;
            let template = match &gateway.ui_origin {
                Some(ui) => template
                    .with_ui_namespace(
                        ui.namespace().as_str(),
                        ui.listener().ok_or_else(|| {
                            AppError::Configuration(String::from("missing UI origin listener"))
                        })?,
                    )
                    .map_err(component("gateway UI Caddy configuration"))?,
                None => template,
            };
            let provider: Arc<dyn gateway_edge::GatewayProvider> = Arc::new(
                LocalCaddyGatewayProvider::new(administration, Arc::clone(&dispatcher))
                    .with_dispatcher_upstream(gateway.dispatcher_listen.to_string())
                    .with_configuration_template(template),
            );
            Some(GatewayEdgeRuntime {
                authority,
                recovery_authority,
                service_supervisor_context,
                service_claim_resolution,
                service_expired_claim_recovery,
                service_boot_context,
                service_log_writer,
                provider,
                dispatcher,
                dispatcher_listen: gateway.dispatcher_listen,
                public_authority: gateway.public_authority,
                ui_dispatcher,
                ui_origin: gateway.ui_origin,
            })
        } else {
            None
        };
        let release_authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
        let release_service = Arc::new(ReleaseService::new(
            pool.clone(),
            release_authorizer.clone(),
        ));
        let artifact_store = LocalArtifactStore::new(
            std::fs::canonicalize(release_artifact_root)
                .map_err(component("release artifact store"))?,
        )
        .map_err(component("release artifact store"))?;
        let build_executor = Arc::new(
            BuildExecutor::initialize(
                Arc::new(PgBuildRepository::new(pool.clone())),
                Arc::clone(&provider),
                artifact_store.clone(),
                Arc::clone(&release_service),
                BuildExecutorConfig {
                    workspace_root: config.build_workspace_root,
                    repository_root: config.repository_root.clone(),
                    git_binary: build_git_binary,
                    image_filesystems: Arc::clone(&image_filesystems),
                    timeout: config.build_timeout,
                },
            )
            .map_err(component("isolated build executor"))?,
        );
        let internal_platform_policy = release_domain::RuntimePolicy {
            vcpus: config.runtime_policy.max_vcpus,
            memory_mib: config.runtime_policy.max_memory_mib,
            network: if config.runtime_policy.allow_egress {
                release_domain::NetworkAccess::Egress
            } else if config.runtime_policy.allow_broker_only {
                release_domain::NetworkAccess::BrokerOnly
            } else {
                release_domain::NetworkAccess::Disabled
            },
        };
        let internal_platform_policy_version = config.runtime_policy.version.clone();
        let spec_factory = Arc::new(PgAgentVmSpecFactory {
            pool: pool.clone(),
            root_images: config.root_images,
            runtime_policy: config.runtime_policy,
        });
        let launch_authorizer =
            Arc::new(PgRunLaunchAuthorizer::new(pool.clone(), release_authorizer));
        let update_completion = Arc::new(UpdateRunCompletion {
            pool: pool.clone(),
            releases: Arc::clone(&release_service),
            admission_cursor: Mutex::new(None),
        });
        let mailbox_store: Arc<dyn MailboxDispatchStore> = mailbox_repository.clone();
        let completion = Arc::new(CompositeRunCompletionObserver::new(vec![
            Arc::clone(&update_completion) as Arc<dyn RunCompletionObserver>,
            Arc::new(MailboxRunCompletion::new(Arc::clone(&mailbox_store))),
        ]));
        let orchestrator = Arc::new(
            RunOrchestrator::new(
                run_repository.clone(),
                volumes,
                Arc::clone(&provider),
                spec_factory,
                config.agent_state_capacity_bytes,
            )
            .with_workspace_manager(
                Arc::clone(&workspaces) as Arc<dyn heph_runtime::RunWorkspaceManager>
            )
            .with_runtime_git_workspace_manager(
                workspaces as Arc<dyn heph_runtime::RuntimeGitWorkspaceManager>,
            )
            .with_runtime_manager(run_runtime)
            .with_launch_authorizer(launch_authorizer)
            .with_resource_observer(Arc::new(MailboxRunResources::new(Arc::clone(
                &mailbox_store,
            ))))
            .with_authority_manager(runtime_authority)
            .with_secret_manager(secret_mounts)
            .with_completion_observer(completion),
        );

        let verifier = Arc::new(OidcVerifier::new(
            config.oidc.issuer,
            &config.oidc.audience,
            config.oidc.algorithm,
            config.oidc.decoding_key,
        ));
        let identity_store = Arc::new(PostgresIdentityStore::new(pool.clone()));
        let oidc_git_authenticator = Arc::new(OidcGitAuthenticator::new(
            verifier,
            Arc::clone(&identity_store) as Arc<dyn identity_application::VerifiedIdentityMapper>,
        ));
        let git_authenticator: Arc<dyn GitAuthenticator> = Arc::new(
            CompositeGitAuthenticator::new(
                oidc_git_authenticator,
                Arc::new(pat_postgres::PostgresPersonalAccessTokenService::new(
                    pool.clone(),
                )),
            )
            .with_runtime_git(Arc::new(RuntimeGitHttpAuthenticator::new(Arc::new(
                runtime_git_credentials,
            )))),
        );
        let git_authorizer = Arc::new(PostgresGitAuthorizer::new(Arc::new(
            authz_postgres::PostgresGitAuthorizer::new(pool.clone()),
        )));
        let nats_client = async_nats::connect(&config.nats_url)
            .await
            .map_err(component("NATS connection"))?;
        let jetstream = async_nats::jetstream::new(nats_client.clone());

        Ok(Self {
            pool,
            application_pool,
            nats_client,
            jetstream,
            forge,
            storage,
            identity_store,
            git_authenticator,
            git_authorizer,
            git_backend: config.git_http_backend,
            git_pre_receive_hook: config.git_pre_receive_hook,
            git_limits: config.git_http_limits,
            registry: config.registry,
            http_listen: config.http_listen,
            service_log_pool,
            run_repository,
            mailbox_repository,
            review_repository,
            review_control,
            orchestrator,
            build_executor,
            oci_builder_workers,
            artifact_store,
            result_artifact_root,
            release_service,
            update_completion,
            secret_service,
            rpc_mediator_signing_key: config.rpc_mediator_signing_key,
            internal_platform_policy,
            internal_platform_policy_version,
            secret_broker_socket: config.secret_broker_socket,
            secret_broker_executor,
            service_log_maintenance,
            gateway_edge,
            worker_concurrency: config.worker_concurrency,
            outbox_poll_interval: config.outbox_poll_interval,
            outbox_batch_size: config.outbox_batch_size,
            startup_timeout: config.startup_timeout,
            shutdown_timeout: config.shutdown_timeout,
            runtime_git_socket_path,
        })
    }

    /// Binds and constructs the optional UI-origin listener before shared
    /// gateway configuration is reconciled.
    async fn build_ui_listener(
        &self,
        git: Arc<GitHttpService>,
    ) -> Result<Option<(tokio::net::TcpListener, Router)>, AppError> {
        ui_listener::build_ui_listener(self, git).await
    }

    /// Binds HTTP, establishes durable NATS topology, and starts supervised
    /// background workers.
    ///
    /// This returns only after HTTP, command consumption, and outbox
    /// publication have crossed their readiness barriers.
    ///
    /// # Errors
    ///
    /// Returns an error when startup or readiness fails. Already-started tasks
    /// are cancelled and reaped before the error is returned.
    pub async fn start(self) -> Result<RunningHephaestus, AppError> {
        // Keep the large startup state machine off the caller's stack.
        Box::pin(self.start_inner()).await
    }

    // Keep readiness barriers and failure cleanup ordered and directly auditable.
    #[allow(clippy::too_many_lines)]
    async fn start_inner(self) -> Result<RunningHephaestus, AppError> {
        self.build_executor
            .recover_after_restart()
            .await
            .map_err(component("build recovery"))?;
        self.orchestrator
            .recover_after_restart()
            .await
            .map_err(component("run recovery"))?;
        ensure_forge_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("forge JetStream topology"))?;
        let build_consumer = ensure_build_consumer(&self.jetstream)
            .await
            .map_err(component("build JetStream consumer"))?;
        ensure_release_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("release JetStream topology"))?;
        event_adapter::ensure_topology(&self.jetstream)
            .await
            .map_err(component("product-event JetStream topology"))?;
        let consumer = ensure_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("run JetStream topology"))?;
        let mailbox_consumer = ensure_mailbox_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("mailbox JetStream topology"))?;
        let receive_hook = self.git_pre_receive_hook.clone();
        let git = Arc::new(
            GitHttpService::new(
                Arc::clone(&self.forge),
                Arc::clone(&self.storage),
                self.git_authenticator.clone(),
                self.git_authorizer.clone(),
                self.git_backend.clone(),
                self.git_limits.clone(),
            )
            .and_then(|service| service.with_runtime_receive_hook(receive_hook))
            .map_err(component("Git HTTP configuration"))?,
        );
        let runtime_git_listener = self
            .runtime_git_socket_path
            .as_ref()
            .map(|path| runtime_git_listener::RuntimeGitListener::bind(path.clone()))
            .transpose()
            .map_err(component("runtime Git Unix listener"))?;
        let runtime_git_router = runtime_git_listener
            .as_ref()
            .map(|_| runtime_git_listener::router(git.as_ref()));
        // Bind the optional UI listener after creating the shared Git service,
        // so browser and public Git requests share repository receive locks.
        let ui_listener = self.build_ui_listener(Arc::clone(&git)).await?;
        let broker = BrokerServer::bind(
            self.secret_broker_socket,
            Arc::clone(&self.secret_broker_executor),
        )
        .map_err(component("secret broker listener"))?;
        let command_state = application::commands::InternalCommandState::new(
            Arc::clone(&self.release_service),
            Arc::clone(&self.secret_service),
            self.internal_platform_policy.clone(),
            self.internal_platform_policy_version.clone(),
        );
        let browser_sessions: Arc<dyn BrowserSessionStore> = Arc::new(
            PostgresBrowserSessionStore::new(self.pool.clone(), self.application_pool.clone()),
        );
        let rpc = rpc::service(
            rpc::ApplicationDependencies::new(
                self.pool.clone(),
                self.application_pool.clone(),
                self.service_log_pool.clone(),
                Arc::clone(&self.forge),
                Arc::new(event_postgres::PostgresMutationReceiptReader::new(
                    self.pool.clone(),
                )),
                Arc::clone(&self.identity_store) as Arc<dyn IdempotentIdentityResolver>,
                Arc::clone(&browser_sessions),
                Arc::clone(&self.release_service),
            ),
            Arc::clone(&self.storage),
            self.artifact_store.clone(),
            self.result_artifact_root,
            &self.rpc_mediator_signing_key,
            command_state,
            Arc::new(event_adapter::NatsEventWakeups::new(
                self.nats_client.clone(),
            )),
        )
        .map_err(component("Connect RPC configuration"))?;
        let ui_request_audit: Arc<dyn release_service::UiRequestAuditSink> = Arc::new(
            release_postgres::PgUiRequestAuditRepository::new(self.service_log_pool.clone()),
        );
        let registry_store = PgRegistryStore::new(self.pool.clone());
        let registry_reconciliation_adapter = PostgresRegistryReconciliation {
            store: registry_store.clone(),
        };
        let registry_reconciler = RegistryReconciler::new(
            registry_reconciliation_adapter.clone(),
            registry_reconciliation_adapter.clone(),
            ZotHttpRegistry::new(
                self.registry.zot.clone(),
                Arc::new(InternalRegistryTokens {
                    issuer: Arc::clone(&self.registry.token_issuer),
                }),
            )
            .map_err(component("Zot reconciliation client"))?,
        );
        let registry_reconciliation_lease = self.registry.reconciliation_lease;
        let registry_reconciliation_interval = self.registry.reconciliation_interval;
        let registry_tokens = RegistryTokenHttpService::new(
            Arc::clone(&self.registry.token_issuer),
            Arc::new(PostgresRegistryScopeAuthorizer {
                store: registry_store.clone(),
            }),
        )
        .router()
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&self.git_authenticator),
            registry_caller_authentication,
        ));
        let registry_notifications = RegistryNotificationHttpService::new(
            self.registry.notification_callback.clone(),
            Arc::new(PostgresRegistryNotificationInbox {
                store: registry_store.clone(),
            }),
        )
        .router();
        let gateway_listener = if let Some(gateway) = &self.gateway_edge {
            let listener = tokio::net::TcpListener::bind(gateway.dispatcher_listen)
                .await
                .map_err(component("gateway private dispatcher listener"))?;
            let desired = gateway
                .authority
                .desired_configuration()
                .await
                .map_err(component("gateway desired configuration"))?;
            gateway
                .provider
                .reconcile(&desired)
                .await
                .map_err(component("gateway Caddy reconciliation"))?;
            Some((
                listener,
                PrivateGatewayDispatcherState {
                    dispatcher: Arc::clone(&gateway.dispatcher),
                    public_authority: gateway.public_authority.clone(),
                },
            ))
        } else {
            None
        };
        let router = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .merge(git.as_ref().clone().router())
            .merge(registry_tokens)
            .merge(registry_notifications)
            .fallback_service(rpc)
            .layer(axum::middleware::from_fn_with_state(
                rpc::MediatorAuthenticationState::new(
                    rpc::MediatorAuthenticator::new(&self.rpc_mediator_signing_key),
                    browser_sessions,
                )
                .with_ui_request_audit_sink(ui_request_audit),
                rpc::mediator_identity_middleware,
            ));
        let listener = tokio::net::TcpListener::bind(self.http_listen)
            .await
            .map_err(component("HTTP listener"))?;
        let http_addr = listener
            .local_addr()
            .map_err(component("HTTP listener address"))?;

        if let Some(workers) = &self.oci_builder_workers {
            workers
                .materialization
                .write_manifest(&workers.manifest)
                .await
                .map_err(component("OCI builder root manifest"))?;
            workers
                .refresh_image_filesystems()
                .await
                .map_err(component("OCI builder image cache"))?;
        }

        let cancellation = CancellationToken::new();
        let mut tasks = Vec::with_capacity(10);
        let service_log_maintenance = Arc::clone(&self.service_log_maintenance);
        let service_log_cancel = cancellation.clone();
        tasks.push(tokio::spawn(async move {
            let result = service_log_maintenance
                .run(service_log_cancel.clone())
                .await
                .map_err(|error| error.to_string());
            if result.is_err() {
                service_log_cancel.cancel();
            }
            result
        }));
        let (update_reconcile_ready_tx, update_reconcile_ready_rx) = oneshot::channel();
        let update_reconcile_cancel = cancellation.clone();
        let update_reconcile_observer = Arc::clone(&self.update_completion);
        let update_reconcile_interval = self.outbox_poll_interval;
        tasks.push(tokio::spawn(async move {
            update_admission_reconciliation_loop(
                update_reconcile_observer,
                update_reconcile_cancel,
                update_reconcile_interval,
                update_reconcile_ready_tx,
            )
            .await;
            Ok(())
        }));
        if let Some(gateway) = self.gateway_edge {
            let gateway_reconcile_cancel = cancellation.clone();
            let gateway_authority = gateway.authority.clone();
            let gateway_recovery_authority = gateway.recovery_authority.clone();
            let service_supervisor_context =
                clone_service_supervisor_context(&gateway.service_supervisor_context);
            let service_boot_recovery =
                GatewayServiceBootRecovery::new(gateway.service_boot_context)
                    .map_err(component("gateway service boot recovery"))?;
            let service_claim_resolution = Arc::clone(&gateway.service_claim_resolution);
            let service_expired_claim_recovery =
                Arc::clone(&gateway.service_expired_claim_recovery);
            let service_log_writer = gateway.service_log_writer;
            let service_targets = Arc::clone(&gateway.service_supervisor_context.targets);
            let gateway_provider = Arc::clone(&gateway.provider);
            tasks.push(tokio::spawn(async move {
                gateway_reconciliation_loop_with_context(
                    gateway_authority,
                    gateway_recovery_authority,
                    service_supervisor_context,
                    service_boot_recovery,
                    Some(service_claim_resolution),
                    Some(service_expired_claim_recovery),
                    Some(service_log_writer),
                    service_targets,
                    gateway_provider,
                    gateway_reconcile_cancel,
                )
                .await;
                Ok(())
            }));
        }
        let (broker_ready_tx, broker_ready_rx) = oneshot::channel();
        let broker_cancel = cancellation.clone();
        tasks.push(tokio::spawn(async move {
            if broker_ready_tx.send(()).is_err() {
                return Ok(());
            }
            let result = broker
                .serve(broker_cancel.clone())
                .await
                .map_err(|error| error.to_string());
            if result.is_err() {
                broker_cancel.cancel();
            }
            result
        }));
        let gateway_ready_rx = if let Some((listener, state)) = gateway_listener {
            let (gateway_ready_tx, gateway_ready_rx) = oneshot::channel();
            let gateway_cancel = cancellation.clone();
            tasks.push(tokio::spawn(async move {
                if gateway_ready_tx.send(()).is_err() {
                    return Ok(());
                }
                let result = axum::serve(
                    listener,
                    Router::new()
                        .fallback(any(private_gateway_dispatch))
                        .with_state(state)
                        .into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(gateway_cancel.clone().cancelled_owned())
                .await
                .map_err(|error| error.to_string());
                if !gateway_cancel.is_cancelled() {
                    gateway_cancel.cancel();
                }
                result
            }));
            Some(gateway_ready_rx)
        } else {
            None
        };
        let ui_ready_rx = if let Some((listener, router)) = ui_listener {
            let (ui_ready_tx, ui_ready_rx) = oneshot::channel();
            let ui_cancel = cancellation.clone();
            tasks.push(tokio::spawn(async move {
                if ui_ready_tx.send(()).is_err() {
                    return Ok(());
                }
                let result = axum::serve(
                    listener,
                    router.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(ui_cancel.clone().cancelled_owned())
                .await
                .map_err(|error| error.to_string());
                if !ui_cancel.is_cancelled() {
                    ui_cancel.cancel();
                }
                result
            }));
            Some(ui_ready_rx)
        } else {
            None
        };
        let runtime_git_ready_rx =
            runtime_git_listener
                .zip(runtime_git_router)
                .map(|(listener, router)| {
                    spawn_runtime_git_listener(listener, router, &cancellation, &mut tasks)
                });
        let (http_ready_tx, http_ready_rx) = oneshot::channel();
        let http_cancel = cancellation.clone();
        tasks.push(tokio::spawn(async move {
            if http_ready_tx.send(()).is_err() {
                return Ok(());
            }
            let graceful = http_cancel.clone();
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(graceful.cancelled_owned())
                .await
                .map_err(|error| error.to_string());
            if !http_cancel.is_cancelled() {
                http_cancel.cancel();
            }
            result
        }));

        let (publisher_ready_tx, publisher_ready_rx) = oneshot::channel();
        let publisher_cancel = cancellation.clone();
        let outbox = OutboxWorker {
            forge_publisher: ForgeNatsOutboxPublisher::new(self.jetstream.clone()),
            release_publisher: ReleaseOutboxPublisher::new(
                self.jetstream.clone(),
                self.pool.clone(),
            ),
            review_publisher: ReviewOutboxPublisher::new(
                self.jetstream.clone(),
                Arc::clone(&self.review_repository) as Arc<dyn review_service::ReviewOutboxStore>,
            ),
            event_publisher: event_adapter::EventPublisher::new(
                self.jetstream.clone(),
                Arc::new(event_postgres::PostgresProductEventOutbox::new(
                    self.pool.clone(),
                )),
                self.rpc_mediator_signing_key,
            ),
            mailbox_publisher: MailboxOutboxPublisher::new(
                self.jetstream.clone(),
                self.mailbox_repository.clone(),
            ),
            forge: Arc::clone(&self.forge),
            poll_interval: self.outbox_poll_interval,
            batch_size: self.outbox_batch_size,
        };
        tasks.push(tokio::spawn(async move {
            outbox.run(publisher_cancel, publisher_ready_tx).await;
            Ok(())
        }));

        let (mailbox_recovery_ready_tx, mailbox_recovery_ready_rx) = oneshot::channel();
        let mailbox_recovery_cancel = cancellation.clone();
        let mailbox_recovery_store: Arc<dyn MailboxDispatchStore> = self.mailbox_repository.clone();
        let mailbox_recovery_interval = self.outbox_poll_interval;
        tasks.push(tokio::spawn(async move {
            let result = mailbox_recovery_loop(
                mailbox_recovery_store,
                mailbox_recovery_interval,
                mailbox_recovery_cancel.clone(),
                mailbox_recovery_ready_tx,
            )
            .await;
            if result.is_err() {
                mailbox_recovery_cancel.cancel();
            }
            result
        }));

        let (secret_reconcile_ready_tx, secret_reconcile_ready_rx) = oneshot::channel();
        let secret_reconcile_cancel = cancellation.clone();
        let secret_reconcile_pool = self.pool.clone();
        let secret_reconcile_orchestrator = Arc::clone(&self.orchestrator);
        let secret_reconcile_interval = self.outbox_poll_interval;
        tasks.push(tokio::spawn(async move {
            let result = secret_revocation_loop(
                secret_reconcile_pool,
                secret_reconcile_orchestrator,
                secret_reconcile_interval,
                secret_reconcile_cancel.clone(),
                secret_reconcile_ready_tx,
            )
            .await;
            if result.is_err() {
                secret_reconcile_cancel.cancel();
            }
            result
        }));

        let (mailbox_consumer_ready_tx, mailbox_consumer_ready_rx) = oneshot::channel();
        let mailbox_consumer_cancel = cancellation.clone();
        let mailbox_handler = NatsMailboxCommandHandler::new(MailboxCommandHandler::new(
            self.mailbox_repository.clone(),
            Arc::clone(&self.orchestrator),
        ));
        let mailbox_concurrency = self.worker_concurrency;
        tasks.push(tokio::spawn(async move {
            let result = mailbox_command_loop(
                mailbox_consumer,
                mailbox_handler,
                mailbox_concurrency,
                mailbox_consumer_cancel.clone(),
                mailbox_consumer_ready_tx,
            )
            .await;
            if result.is_err() {
                mailbox_consumer_cancel.cancel();
            }
            result
        }));

        let (build_ready_tx, build_ready_rx) = oneshot::channel();
        let build_cancel = cancellation.clone();
        let build_executor = Arc::clone(&self.build_executor);
        let build_concurrency = self.worker_concurrency;
        tasks.push(tokio::spawn(async move {
            let result = build_loop(
                build_consumer,
                build_executor,
                build_concurrency,
                build_cancel.clone(),
                build_ready_tx,
            )
            .await;
            if result.is_err() {
                build_cancel.cancel();
            }
            result
        }));

        if let Some(workers) = &self.oci_builder_workers {
            let oci_workers = Arc::clone(workers);
            let oci_cancel = cancellation.clone();
            tasks.push(tokio::spawn(async move {
                oci_builder_loop(oci_workers, oci_cancel).await;
                Ok(())
            }));
        }

        let registry_reconcile_cancel = cancellation.clone();
        tasks.push(tokio::spawn(async move {
            registry_reconciliation_loop(
                registry_reconciler,
                registry_reconciliation_adapter,
                registry_reconciliation_lease,
                registry_reconciliation_interval,
                registry_reconcile_cancel,
            )
            .await;
            Ok(())
        }));

        let (consumer_ready_tx, consumer_ready_rx) = oneshot::channel();
        let consumer_cancel = cancellation.clone();
        let handler = NatsCommandHandler::new(Arc::clone(&self.orchestrator));
        let control_handler = NatsControlHandler::new(self.review_control);
        let concurrency = self.worker_concurrency;
        tasks.push(tokio::spawn(async move {
            let result = command_loop(
                consumer,
                handler,
                control_handler,
                concurrency,
                consumer_cancel.clone(),
                consumer_ready_tx,
            )
            .await;
            if result.is_err() {
                consumer_cancel.cancel();
            }
            result
        }));

        let readiness = async {
            broker_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("secret broker task exited")))?;
            update_reconcile_ready_rx.await.map_err(|_| {
                AppError::Readiness(String::from("update reconciliation task exited"))
            })?;
            http_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("HTTP task exited")))?;
            if let Some(gateway_ready_rx) = gateway_ready_rx {
                gateway_ready_rx.await.map_err(|_| {
                    AppError::Readiness(String::from("gateway private dispatcher task exited"))
                })?;
            }
            if let Some(ui_ready_rx) = ui_ready_rx {
                ui_ready_rx.await.map_err(|_| {
                    AppError::Readiness(String::from("UI origin listener task exited"))
                })?;
            }
            if let Some(runtime_git_ready_rx) = runtime_git_ready_rx {
                runtime_git_ready_rx.await.map_err(|_| {
                    AppError::Readiness(String::from("runtime Git listener task exited"))
                })?;
            }
            publisher_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("outbox task exited")))?;
            mailbox_recovery_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("mailbox recovery task exited")))?;
            secret_reconcile_ready_rx.await.map_err(|_| {
                AppError::Readiness(String::from("secret reconciliation task exited"))
            })?;
            build_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("build task exited")))?;
            consumer_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("consumer task exited")))?;
            mailbox_consumer_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("mailbox consumer task exited")))?;
            Ok::<(), AppError>(())
        };
        match tokio::time::timeout(self.startup_timeout, readiness).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                cancellation.cancel();
                reap_failed_start(tasks).await;
                return Err(error);
            }
            Err(error) => {
                cancellation.cancel();
                reap_failed_start(tasks).await;
                return Err(AppError::Readiness(format!(
                    "startup readiness timed out: {error}"
                )));
            }
        }
        if cancellation.is_cancelled() {
            reap_failed_start(tasks).await;
            return Err(AppError::Readiness(String::from(
                "a supervised task exited during startup",
            )));
        }

        Ok(RunningHephaestus {
            http_addr,
            cancellation,
            tasks,
            pool: self.pool,
            application_pool: self.application_pool,
            service_log_pool: self.service_log_pool,
            nats_client: self.nats_client,
            jetstream: self.jetstream,
            forge: self.forge,
            run_repository: self.run_repository,
            mailbox_repository: self.mailbox_repository,
            review_repository: self.review_repository,
            orchestrator: self.orchestrator,
            outbox_batch_size: self.outbox_batch_size,
            product_event_cursor_key: self.rpc_mediator_signing_key,
            shutdown_timeout: self.shutdown_timeout,
        })
    }
}

/// Running daemon handle.
pub struct RunningHephaestus {
    http_addr: SocketAddr,
    cancellation: CancellationToken,
    tasks: Vec<JoinHandle<Result<(), String>>>,
    pool: PgPool,
    application_pool: PgPool,
    service_log_pool: PgPool,
    nats_client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
    forge: Arc<PgForgeRepository>,
    run_repository: Arc<PgRunRepository>,
    mailbox_repository: Arc<PostgresMailboxRepository>,
    review_repository: Arc<PostgresReviewRepository>,
    orchestrator: Arc<RunOrchestrator>,
    outbox_batch_size: i64,
    product_event_cursor_key: [u8; 32],
    shutdown_timeout: Duration,
}

impl RunningHephaestus {
    /// Bound HTTP address after the readiness barrier.
    #[must_use]
    pub const fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    /// Returns a retained application-role pool for daemon integration checks.
    #[cfg(feature = "test-fixtures")]
    #[doc(hidden)]
    #[must_use]
    pub fn application_pool_for_test(&self) -> ControlPlanePool {
        self.application_pool.clone()
    }

    /// Returns the dedicated worker pool used by the service-log scheduler.
    #[cfg(feature = "test-fixtures")]
    #[doc(hidden)]
    #[must_use]
    pub fn service_log_pool_for_test(&self) -> ControlPlanePool {
        self.service_log_pool.clone()
    }

    /// Waits for one persisted lifecycle event.
    ///
    /// # Errors
    ///
    /// Returns an error for database failure or timeout.
    pub async fn wait_for_run_event(
        &self,
        run_id: RunId,
        kind: RunEventKind,
        timeout: Duration,
    ) -> Result<(), AppError> {
        let deadline = Instant::now() + timeout;
        loop {
            let exists =
                control_plane_postgres::has_run_event(&self.pool, run_id.as_uuid(), kind.as_str())
                    .await
                    .map_err(component("run event query"))?;
            if exists {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(AppError::Timeout(format!(
                    "run {run_id} did not persist {}",
                    kind.as_str()
                )));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Stops admission, cancels active runs, drains supervised tasks, and
    /// closes NATS and `PostgreSQL` resources.
    ///
    /// # Errors
    ///
    /// Returns the first task or resource-shutdown failure.
    pub async fn shutdown(mut self) -> Result<(), AppError> {
        self.cancellation.cancel();
        for run in self
            .run_repository
            .recoverable_runs()
            .await
            .map_err(component("recoverable run query"))?
        {
            let command = CancelRun {
                command_id: CommandId::new(),
                run_id: run.id,
                reason: String::from("daemon shutdown"),
            };
            if let Err(error) = self.orchestrator.cancel_run(&command).await {
                tracing::warn!(run_id = %run.id, %error, "active run cancellation failed");
            }
        }

        let deadline = Instant::now() + self.shutdown_timeout;
        let mut first_error = None;
        for mut task in self.tasks.drain(..) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => {
                    first_error.get_or_insert(AppError::Task(error));
                }
                Ok(Err(error)) => {
                    first_error.get_or_insert_with(|| AppError::Task(error.to_string()));
                }
                Err(_) => {
                    task.abort();
                    drop(task.await);
                    first_error.get_or_insert_with(|| {
                        AppError::Timeout(String::from("supervised task drain timed out"))
                    });
                }
            }
        }
        if let Err(error) = self.flush_outbox(deadline).await {
            first_error.get_or_insert(error);
        }
        if let Err(error) = self.nats_client.drain().await {
            first_error.get_or_insert_with(|| AppError::Shutdown(error.to_string()));
        }
        self.pool.close().await;
        self.application_pool.close().await;
        self.service_log_pool.close().await;
        first_error.map_or(Ok(()), Err)
    }

    async fn flush_outbox(&self, deadline: Instant) -> Result<(), AppError> {
        let forge_publisher = ForgeNatsOutboxPublisher::new(self.jetstream.clone());
        let release_publisher =
            ReleaseOutboxPublisher::new(self.jetstream.clone(), self.pool.clone());
        let review_publisher = ReviewOutboxPublisher::new(
            self.jetstream.clone(),
            Arc::clone(&self.review_repository) as Arc<dyn review_service::ReviewOutboxStore>,
        );
        let event_publisher = event_adapter::EventPublisher::new(
            self.jetstream.clone(),
            Arc::new(event_postgres::PostgresProductEventOutbox::new(
                self.pool.clone(),
            )),
            self.product_event_cursor_key,
        );
        let mailbox_publisher =
            MailboxOutboxPublisher::new(self.jetstream.clone(), self.mailbox_repository.clone());
        let diagnostics = Arc::new(StdMutex::new(FlushDiagnostics::new(deadline)));
        let result = flush_until_quiescent(deadline, Arc::clone(&diagnostics), || {
            let diagnostics = Arc::clone(&diagnostics);
            let forge_publisher = forge_publisher.clone();
            let release_publisher = release_publisher.clone();
            let review_publisher = review_publisher.clone();
            let event_publisher = event_publisher.clone();
            let mailbox_publisher = mailbox_publisher.clone();
            async move {
                let forge = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Forge,
                    forge_publisher.publish_pending(self.forge.as_ref(), self.outbox_batch_size),
                    "final forge outbox flush",
                )
                .await?;
                let releases = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Release,
                    release_publisher.publish_pending(self.outbox_batch_size),
                    "final release outbox flush",
                )
                .await?;
                let reviews = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Review,
                    review_publisher.publish_pending(self.outbox_batch_size),
                    "final review outbox flush",
                )
                .await?;
                let events = flush_publisher(
                    &diagnostics,
                    FlushPublisher::ProductEvent,
                    event_publisher.publish_pending(self.outbox_batch_size),
                    "final product-event outbox flush",
                )
                .await?;
                let mailboxes = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Mailbox,
                    mailbox_publisher.publish_pending(self.outbox_batch_size),
                    "final mailbox outbox flush",
                )
                .await?;
                Ok::<_, AppError>(
                    forge == 0 && releases == 0 && reviews == 0 && events == 0 && mailboxes == 0,
                )
            }
        })
        .await;
        if result.is_err() {
            diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned")
                .log_failure(deadline);
        }
        result
    }
}

async fn verify_database_contract(pool: &PgPool) -> Result<(), AppError> {
    let (migration, melange) = control_plane_postgres::verify_contract(pool)
        .await
        .map_err(component("database contract check"))?;
    if migration != Some(EXPECTED_DATABASE_MIGRATION) {
        return Err(AppError::Configuration(format!(
            "database migration is {migration:?}; expected {EXPECTED_DATABASE_MIGRATION}"
        )));
    }
    if !melange {
        return Err(AppError::Configuration(String::from(
            "Mélange check_permission dispatcher is missing",
        )));
    }
    Ok(())
}

fn component<Error: std::fmt::Display>(
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
    #[error("application shutdown failed: {0}")]
    Shutdown(String),
}

#[cfg(test)]
mod gateway_recovery_tests;

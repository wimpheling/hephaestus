//! Production composition root and supervised daemon lifecycle.

mod application;
mod event_adapter;
mod event_cursor;
pub mod rpc;

use async_trait::async_trait;
use axum::{Router, routing::get};
use build_orchestrator::{
    BuildExecutionError, BuildExecutor, BuildExecutorConfig, CatalogBuilderImageResolver,
};
use build_postgres::PgBuildRepository;
use builder_catalog_postgres::PgBuilderCatalog;
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use control_plane_postgres::{
    ControlPlanePool, connect as connect_control_plane, load_vm_launch_contract,
    recoverable_update_hook_run_ids,
};
use event_postgres::{ReleaseOutboxPublisher, ensure_release_jetstream_topology};
use forge_postgres::PgForgeRepository;
use forge_service::{
    BUILD_REQUESTED_SUBJECT, BUILD_RETRY_REQUESTED_SUBJECT, BUILD_VERIFY_REQUESTED_SUBJECT,
    ForgeNatsOutboxPublisher, GitStorage, ensure_build_consumer, ensure_forge_jetstream_topology,
};
use futures_util::StreamExt;
use git_http::{GitHttpLimits, GitHttpService, OidcGitAuthenticator, PostgresGitAuthorizer};
use identity_application::IdempotentIdentityResolver;
use identity_oidc::OidcVerifier;
use identity_postgres::PostgresIdentityStore;
use jsonwebtoken::{Algorithm, DecodingKey};
use release_artifact_store::LocalArtifactStore;
use release_domain::BuildRequestId;
use release_postgres::ReleaseService;
use review_domain::CONTROL_EXECUTE_SUBJECT;
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::{NatsControlHandler, ReviewControlService, ReviewOutboxPublisher};
use run_domain::{CancelRun, Run, RunKind};
use run_orchestrator::{
    NatsCommandHandler, RunCompletionError, RunCompletionObserver, RunOrchestrator, RunRepository,
    RunSecretManager, VmSpecFactory, ensure_jetstream_topology,
};
use run_postgres::PgRunRepository;
use run_runtime_local::{LocalRunRuntimeConfig, LocalRunRuntimeManager};
use runtime_types::{CommandId, RunId};
use secret_application::BrokerAdapter;
use secret_broker::{BrokerExecutor, BrokerServer, ServiceBrokerExecutor};
use secret_postgres::initialize_manager;
use secret_postgres::{SecretRuntimeService, SecretService};
use secret_runtime::EphemeralSecretConfig;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde::Deserialize;
type PgPool = ControlPlanePool;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{Semaphore, broadcast, oneshot, watch},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vm_fake::FakeProvider;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId,
    VmInstance, VmMetric, VmMount, VmProvider, VmResources, VmSpec,
};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_postgres::PostgresVolumeMetadataRepository;
use workspace_local::{LocalWorkspaceConfig, LocalWorkspaceManager};
use workspace_postgres::PgWorkspaceMetadataRepository;

/// Ordered database migration expected by this application version.
pub const EXPECTED_DATABASE_MIGRATION: i64 = 15;

/// OIDC issuer configuration used for bearer-token authentication.
pub struct OidcConfig {
    /// Trusted issuer URL.
    pub issuer: String,
    /// Required token audience.
    pub audience: String,
    /// Expected JWT signature algorithm.
    pub algorithm: Algorithm,
    /// Trusted decoding key resolved from the issuer's JWKS.
    pub decoding_key: DecodingKey,
}

/// Configured VM backend.
pub enum VmBackendConfig {
    /// Deterministic development and test provider.
    Fake,
    /// Deterministic local E2E guest which edits and finalizes its workspace.
    FixtureResult,
    /// Explicit provider injection for hardware-independent end-to-end tests.
    Custom(Arc<dyn VmProvider>),
    /// Production libkrun provider.
    Libkrun(Box<LibkrunConfig>),
}

/// Current platform ceiling applied again immediately before every guest boot.
#[derive(Clone, Debug)]
pub struct RuntimePolicy {
    /// Operator-defined policy revision recorded on launched VMs.
    pub version: String,
    /// Largest permitted virtual CPU allocation.
    pub max_vcpus: u8,
    /// Largest permitted memory allocation, in mebibytes.
    pub max_memory_mib: u32,
    /// Whether guests may use only the semantic secret broker transport.
    pub allow_broker_only: bool,
    /// Whether guests may receive general outbound user-mode networking.
    pub allow_egress: bool,
}

/// Complete configuration consumed by the composition root.
pub struct AppConfig {
    /// Runtime `PostgreSQL` connection string.
    pub database_url: String,
    /// NATS server connection string.
    pub nats_url: String,
    /// HTTP address for the API and Git transport.
    pub http_listen: SocketAddr,
    /// Domain-separated HS256 key for short-lived mediator assertions.
    pub rpc_mediator_signing_key: [u8; 32],
    /// Canonical root containing bare repositories.
    pub repository_root: PathBuf,
    /// Absolute native `git-http-backend` executable.
    pub git_http_backend: PathBuf,
    /// Git transaction limits.
    pub git_http_limits: GitHttpLimits,
    /// OIDC verifier settings.
    pub oidc: OidcConfig,
    /// Local persistent-volume settings.
    pub volumes: LocalVolumeConfig,
    /// Exact-commit workspace and durable result storage settings.
    pub workspaces: LocalWorkspaceConfig,
    /// Exact release-artifact and host-context runtime filesystem settings.
    pub run_runtime: LocalRunRuntimeConfig,
    /// Private transient root for isolated build source and output trees.
    pub build_workspace_root: PathBuf,
    /// Maximum wall-clock time for one isolated build.
    pub build_timeout: Duration,
    /// Memory-backed ephemeral secret mount settings.
    pub secret_mounts: EphemeralSecretConfig,
    /// Host-loaded versioned wrapping keys.
    pub secret_keys: LocalKeyProvider,
    /// Private provider-facing semantic broker socket.
    pub secret_broker_socket: PathBuf,
    /// Host-only semantic provider adapter.
    pub secret_broker_adapter: Arc<dyn BrokerAdapter>,
    /// VM implementation selected for this process.
    pub vm_backend: VmBackendConfig,
    /// Immutable image references resolved to provider-neutral roots.
    pub root_images: BTreeMap<String, RootFilesystem>,
    /// Current launch-time resource and network ceiling.
    pub runtime_policy: RuntimePolicy,
    /// State-volume capacity provisioned per agent.
    pub agent_state_capacity_bytes: u64,
    /// Maximum concurrently handled NATS commands.
    pub worker_concurrency: usize,
    /// Outbox polling interval.
    pub outbox_poll_interval: Duration,
    /// Records processed by each publisher pass.
    pub outbox_batch_size: i64,
    /// Maximum time allowed for readiness.
    pub startup_timeout: Duration,
    /// Maximum time allowed for graceful task draining.
    pub shutdown_timeout: Duration,
}

impl AppConfig {
    fn validate(&self) -> Result<(), AppError> {
        if !self.git_http_backend.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "git_http_backend must be absolute",
            )));
        }
        if self.rpc_mediator_signing_key == [0; 32] {
            return Err(AppError::Configuration(String::from(
                "RPC mediator authentication key must not be all-zero",
            )));
        }
        if !self.secret_broker_socket.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "secret_broker_socket must be absolute",
            )));
        }
        let backend = std::fs::metadata(&self.git_http_backend).map_err(|error| {
            AppError::Configuration(format!("git_http_backend cannot be inspected: {error}"))
        })?;
        if !backend.is_file() || backend.permissions().mode() & 0o111 == 0 {
            return Err(AppError::Configuration(String::from(
                "git_http_backend must be an executable file",
            )));
        }
        if !self.repository_root.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "repository_root must be absolute",
            )));
        }
        if !self.build_workspace_root.is_absolute() || self.build_timeout.is_zero() {
            return Err(AppError::Configuration(String::from(
                "isolated build root must be absolute and timeout must be positive",
            )));
        }
        if self.root_images.is_empty() {
            return Err(AppError::Configuration(String::from(
                "at least one root image mapping is required",
            )));
        }
        if self.runtime_policy.version.trim().is_empty()
            || self.runtime_policy.max_vcpus == 0
            || self.runtime_policy.max_memory_mib == 0
        {
            return Err(AppError::Configuration(String::from(
                "runtime policy version and positive resource ceilings are required",
            )));
        }
        if self.worker_concurrency == 0 {
            return Err(AppError::Configuration(String::from(
                "worker_concurrency must be greater than zero",
            )));
        }
        if self.outbox_batch_size <= 0 {
            return Err(AppError::Configuration(String::from(
                "outbox_batch_size must be greater than zero",
            )));
        }
        if self.startup_timeout.is_zero() || self.shutdown_timeout.is_zero() {
            return Err(AppError::Configuration(String::from(
                "startup and shutdown timeouts must be greater than zero",
            )));
        }
        Ok(())
    }
}

/// Constructed application whose external tasks have not started.
pub struct HephaestusApp {
    pool: PgPool,
    nats_client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
    forge: Arc<PgForgeRepository>,
    storage: Arc<GitStorage>,
    identity_store: Arc<PostgresIdentityStore>,
    git_authenticator: Arc<OidcGitAuthenticator>,
    git_authorizer: Arc<PostgresGitAuthorizer>,
    git_backend: PathBuf,
    git_limits: GitHttpLimits,
    http_listen: SocketAddr,
    run_repository: Arc<PgRunRepository>,
    review_repository: Arc<PostgresReviewRepository>,
    review_control: ReviewControlService,
    orchestrator: Arc<RunOrchestrator>,
    build_executor: Arc<BuildExecutor>,
    artifact_store: LocalArtifactStore,
    result_artifact_root: PathBuf,
    release_service: Arc<ReleaseService>,
    secret_service: Arc<SecretService<LocalKeyProvider>>,
    rpc_mediator_signing_key: [u8; 32],
    internal_platform_policy: release_domain::RuntimePolicy,
    internal_platform_policy_version: String,
    secret_broker_socket: PathBuf,
    secret_broker_executor: Arc<dyn BrokerExecutor>,
    worker_concurrency: usize,
    outbox_poll_interval: Duration,
    outbox_batch_size: i64,
    startup_timeout: Duration,
    shutdown_timeout: Duration,
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
        config.validate()?;
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
            Arc::clone(&workspace_repository)
                as Arc<dyn workspace_domain::WorkspaceMetadataRepository>,
            Arc::clone(&workspace_repository) as Arc<dyn workspace_domain::ResultRepository>,
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
        let (secret_mounts, secret_runtime, secret_service) = build_secret_mount_manager(
            pool.clone(),
            &config.database_url,
            config.secret_keys,
            config.secret_mounts,
        )
        .await?;
        let secret_broker_executor: Arc<dyn BrokerExecutor> = Arc::new(ServiceBrokerExecutor::new(
            secret_runtime,
            config.secret_broker_adapter,
        ));

        let provider: Arc<dyn VmProvider> = match config.vm_backend {
            VmBackendConfig::Fake => Arc::new(FakeProvider::new()),
            VmBackendConfig::FixtureResult => Arc::new(ResultFixtureProvider),
            VmBackendConfig::Custom(provider) => provider,
            VmBackendConfig::Libkrun(provider) => {
                Arc::new(LibkrunProvider::new(*provider).map_err(component("libkrun provider"))?)
            }
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
        let builder_catalog = Arc::new(PgBuilderCatalog::new(pool.clone()));
        let build_root_resolver = Arc::new(CatalogBuilderImageResolver::new(
            builder_catalog,
            config.root_images.clone(),
        ));
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
                    root_images: config.root_images.clone(),
                    timeout: config.build_timeout,
                },
            )
            .map_err(component("isolated build executor"))?
            .with_root_image_resolver(build_root_resolver),
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
        let completion = Arc::new(UpdateRunCompletion {
            pool: pool.clone(),
            releases: Arc::clone(&release_service),
        });
        let orchestrator = Arc::new(
            RunOrchestrator::new(
                run_repository.clone(),
                volumes,
                Arc::clone(&provider),
                spec_factory,
                config.agent_state_capacity_bytes,
            )
            .with_workspace_manager(workspaces)
            .with_runtime_manager(run_runtime)
            .with_launch_authorizer(launch_authorizer)
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
        let git_authenticator = Arc::new(OidcGitAuthenticator::new(
            verifier,
            Arc::clone(&identity_store) as Arc<dyn identity_application::VerifiedIdentityMapper>,
        ));
        let git_authorizer = Arc::new(PostgresGitAuthorizer::new(Arc::new(
            authz_postgres::PostgresGitAuthorizer::new(pool.clone()),
        )));
        let nats_client = async_nats::connect(&config.nats_url)
            .await
            .map_err(component("NATS connection"))?;
        let jetstream = async_nats::jetstream::new(nats_client.clone());

        Ok(Self {
            pool,
            nats_client,
            jetstream,
            forge,
            storage,
            identity_store,
            git_authenticator,
            git_authorizer,
            git_backend: config.git_http_backend,
            git_limits: config.git_http_limits,
            http_listen: config.http_listen,
            run_repository,
            review_repository,
            review_control,
            orchestrator,
            build_executor,
            artifact_store,
            result_artifact_root,
            release_service,
            secret_service,
            rpc_mediator_signing_key: config.rpc_mediator_signing_key,
            internal_platform_policy,
            internal_platform_policy_version,
            secret_broker_socket: config.secret_broker_socket,
            secret_broker_executor,
            worker_concurrency: config.worker_concurrency,
            outbox_poll_interval: config.outbox_poll_interval,
            outbox_batch_size: config.outbox_batch_size,
            startup_timeout: config.startup_timeout,
            shutdown_timeout: config.shutdown_timeout,
        })
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
    // Startup deliberately remains ordered in one method so the readiness
    // barrier and failure cleanup sequence are directly auditable.
    #[allow(clippy::too_many_lines)]
    pub async fn start(self) -> Result<RunningHephaestus, AppError> {
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
        let broker = BrokerServer::bind(
            self.secret_broker_socket,
            Arc::clone(&self.secret_broker_executor),
        )
        .map_err(component("secret broker listener"))?;
        let git = GitHttpService::new(
            Arc::clone(&self.forge),
            Arc::clone(&self.storage),
            self.git_authenticator,
            self.git_authorizer,
            self.git_backend,
            self.git_limits,
        )
        .map_err(component("Git HTTP configuration"))?;
        let command_state = application::commands::InternalCommandState::new(
            Arc::clone(&self.release_service),
            Arc::clone(&self.secret_service),
            self.internal_platform_policy.clone(),
            self.internal_platform_policy_version.clone(),
        );
        let rpc = rpc::service(
            rpc::ApplicationDependencies::new(
                self.pool.clone(),
                Arc::clone(&self.forge),
                Arc::new(event_postgres::PostgresMutationReceiptReader::new(
                    self.pool.clone(),
                )),
                Arc::clone(&self.identity_store) as Arc<dyn IdempotentIdentityResolver>,
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
        let router = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .merge(git.router())
            .fallback_service(rpc)
            .layer(axum::middleware::from_fn_with_state(
                rpc::MediatorAuthenticator::new(&self.rpc_mediator_signing_key),
                rpc::mediator_identity_middleware,
            ));
        let listener = tokio::net::TcpListener::bind(self.http_listen)
            .await
            .map_err(component("HTTP listener"))?;
        let http_addr = listener
            .local_addr()
            .map_err(component("HTTP listener address"))?;

        let cancellation = CancellationToken::new();
        let mut tasks = Vec::with_capacity(6);
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
            forge: Arc::clone(&self.forge),
            poll_interval: self.outbox_poll_interval,
            batch_size: self.outbox_batch_size,
        };
        tasks.push(tokio::spawn(async move {
            outbox.run(publisher_cancel, publisher_ready_tx).await;
            Ok(())
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
            http_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("HTTP task exited")))?;
            publisher_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("outbox task exited")))?;
            secret_reconcile_ready_rx.await.map_err(|_| {
                AppError::Readiness(String::from("secret reconciliation task exited"))
            })?;
            build_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("build task exited")))?;
            consumer_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("consumer task exited")))?;
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
            nats_client: self.nats_client,
            jetstream: self.jetstream,
            forge: self.forge,
            run_repository: self.run_repository,
            review_repository: self.review_repository,
            orchestrator: self.orchestrator,
            outbox_batch_size: self.outbox_batch_size,
            product_event_cursor_key: self.rpc_mediator_signing_key,
            shutdown_timeout: self.shutdown_timeout,
        })
    }
}

async fn reap_failed_start(tasks: Vec<JoinHandle<Result<(), String>>>) {
    for task in tasks {
        task.abort();
        drop(task.await);
    }
}

struct OutboxWorker {
    forge_publisher: ForgeNatsOutboxPublisher,
    release_publisher: ReleaseOutboxPublisher,
    review_publisher: ReviewOutboxPublisher,
    event_publisher: event_adapter::EventPublisher,
    forge: Arc<PgForgeRepository>,
    poll_interval: Duration,
    batch_size: i64,
}

impl OutboxWorker {
    // Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
    // redundant public crate visibility.
    #[allow(clippy::redundant_pub_crate)]
    // The explicit worker fan-out keeps each durable outbox and its failure
    // policy visible in one supervised loop.
    #[allow(clippy::cognitive_complexity)]
    async fn run(self, cancellation: CancellationToken, ready: oneshot::Sender<()>) {
        let mut interval = tokio::time::interval(self.poll_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        if ready.send(()).is_err() {
            return;
        }
        loop {
            tokio::select! {
                () = cancellation.cancelled() => break,
                _ = interval.tick() => {
                    if let Err(error) = self
                        .forge_publisher
                        .publish_pending(self.forge.as_ref(), self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "forge outbox publication pass failed");
                    }
                    if let Err(error) = self
                        .release_publisher
                        .publish_pending(self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "release outbox publication pass failed");
                    }
                    if let Err(error) = self
                        .review_publisher
                        .publish_pending(self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "review outbox publication pass failed");
                    }
                    if let Err(error) = self.event_publisher.publish_pending(self.batch_size).await {
                        tracing::warn!(%error, "product-event outbox publication pass failed");
                    }
                }
            }
        }
    }
}

async fn secret_revocation_loop(
    pool: PgPool,
    orchestrator: Arc<RunOrchestrator>,
    poll_interval: Duration,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    reconcile_revoked_raw_runs(&pool, orchestrator.as_ref()).await?;
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            () = tokio::time::sleep(poll_interval) => {
                reconcile_revoked_raw_runs(&pool, orchestrator.as_ref()).await?;
            }
        }
    }
}

async fn reconcile_revoked_raw_runs(
    pool: &PgPool,
    canceller: &(impl RevokedRawRunCanceller + ?Sized),
) -> Result<usize, String> {
    let run_ids = control_plane_postgres::revoked_raw_run_ids(pool)
        .await
        .map_err(|error| error.to_string())?;
    let mut cancellation_count = 0;
    for run_id in run_ids {
        let run_id = RunId::from_uuid(run_id);
        if canceller.cancel_revoked_raw_run(run_id).await? {
            cancellation_count += 1;
        }
    }
    Ok(cancellation_count)
}

#[async_trait]
trait RevokedRawRunCanceller: Sync {
    async fn cancel_revoked_raw_run(&self, run_id: RunId) -> Result<bool, String>;
}

#[async_trait]
impl RevokedRawRunCanceller for RunOrchestrator {
    async fn cancel_revoked_raw_run(&self, run_id: RunId) -> Result<bool, String> {
        self.cancel_run(&CancelRun {
            command_id: CommandId::new(),
            run_id,
            reason: String::from("raw secret authority was revoked"),
        })
        .await
        .map_err(|error| error.to_string())
    }
}

async fn build_secret_mount_manager(
    pool: PgPool,
    database_url: &str,
    keys: LocalKeyProvider,
    config: EphemeralSecretConfig,
) -> Result<
    (
        Arc<dyn RunSecretManager>,
        Arc<SecretRuntimeService<LocalKeyProvider>>,
        Arc<SecretService<LocalKeyProvider>>,
    ),
    AppError,
> {
    let resolver_pool = connect_control_plane(database_url, 4)
        .await
        .map_err(component("secret resolver PostgreSQL connection"))?;
    let authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
    let dispatch = Arc::new(SecretService::new(
        pool.clone(),
        EncryptedStore::new(keys.clone()),
        authorizer.clone(),
    ));
    let runtime = Arc::new(SecretRuntimeService::new(
        pool.clone(),
        resolver_pool,
        EncryptedStore::new(keys),
        authorizer,
    ));
    let manager = initialize_manager(
        pool,
        dispatch.as_ref().clone(),
        runtime.as_ref().clone(),
        config,
    )
    .map_err(component("secret mount initialization"))?;
    Ok((Arc::new(manager), runtime, dispatch))
}

struct UpdateRunCompletion {
    pool: PgPool,
    releases: Arc<ReleaseService>,
}

impl UpdateRunCompletion {
    async fn apply(&self, run: &Run) -> Result<bool, RunCompletionError> {
        if run.kind != RunKind::Update {
            return Ok(false);
        }
        self.releases
            .reconcile_update_run(run.id)
            .await
            .map_err(completion_error)?;
        Ok(true)
    }
}

#[async_trait]
impl RunCompletionObserver for UpdateRunCompletion {
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError> {
        self.apply(run).await.map(|_| ())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        let run_ids = recoverable_update_hook_run_ids(&self.pool)
            .await
            .map_err(completion_error)?;
        let mut recovered = 0;
        for run_id in run_ids {
            self.releases
                .reconcile_update_run(RunId::from_uuid(run_id))
                .await
                .map_err(completion_error)?;
            recovered += 1;
        }
        Ok(recovered)
    }
}

#[derive(Deserialize)]
struct BuildRequestedPayload {
    build_request_id: Uuid,
}

async fn build_loop(
    consumer: async_nats::jetstream::consumer::PullConsumer,
    executor: Arc<BuildExecutor>,
    concurrency: usize,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    let mut messages = consumer
        .messages()
        .await
        .map_err(|error| error.to_string())?;
    let permits = Arc::new(Semaphore::new(concurrency));
    let mut builds = JoinSet::new();
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            delivery = messages.next() => {
                let Some(delivery) = delivery else {
                    return Err(String::from("build command stream ended"));
                };
                let message = delivery.map_err(|error| error.to_string())?;
                let permit = Arc::clone(&permits)
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let executor = Arc::clone(&executor);
                builds.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_build_message(&executor, &message).await {
                        tracing::warn!(%error, "build command handling failed");
                    }
                });
            }
            result = builds.join_next(), if !builds.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "build command task panicked");
                }
            }
        }
    }
    while let Some(result) = builds.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "build command task panicked while draining");
        }
    }
    Ok(())
}

async fn handle_build_message(
    executor: &BuildExecutor,
    message: &async_nats::jetstream::Message,
) -> Result<(), String> {
    let retry = message.message.subject.as_str() == BUILD_RETRY_REQUESTED_SUBJECT;
    let verify = message.message.subject.as_str() == BUILD_VERIFY_REQUESTED_SUBJECT;
    if message.message.subject.as_str() != BUILD_REQUESTED_SUBJECT && !retry && !verify {
        message
            .ack_with(async_nats::jetstream::AckKind::Term)
            .await
            .map_err(|error| error.to_string())?;
        return Err(String::from("unknown build command subject"));
    }
    let payload: BuildRequestedPayload = match serde_json::from_slice(&message.payload) {
        Ok(payload) => payload,
        Err(error) => {
            message
                .ack_with(async_nats::jetstream::AckKind::Term)
                .await
                .map_err(|ack_error| ack_error.to_string())?;
            return Err(error.to_string());
        }
    };
    let operation = async {
        if verify {
            executor
                .verify(BuildRequestId::from_uuid(payload.build_request_id))
                .await
        } else if retry {
            executor
                .retry(BuildRequestId::from_uuid(payload.build_request_id))
                .await
                .map(|_| ())
        } else {
            executor
                .execute(BuildRequestId::from_uuid(payload.build_request_id))
                .await
                .map(|_| ())
        }
    };
    tokio::pin!(operation);
    let mut progress = tokio::time::interval(Duration::from_secs(10));
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut operation => {
                match result {
                    Ok(()) => {
                        message.double_ack().await.map_err(|error| error.to_string())?;
                        return Ok(());
                    }
                    Err(error) if build_delivery_requires_redelivery(&error) => {
                        return Err(error.to_string());
                    }
                    Err(error) => {
                        message.double_ack().await.map_err(|ack_error| ack_error.to_string())?;
                        return Err(error.to_string());
                    }
                }
            }
            _ = progress.tick() => {
                if let Err(error) = message
                    .ack_with(async_nats::jetstream::AckKind::Progress)
                    .await
                {
                    tracing::warn!(%error, "failed to acknowledge build progress");
                }
            }
        }
    }
}

const fn build_delivery_requires_redelivery(error: &BuildExecutionError) -> bool {
    matches!(
        error,
        BuildExecutionError::Database(_)
            | BuildExecutionError::Release
            | BuildExecutionError::VmCleanup
    )
}

fn completion_error(error: impl std::fmt::Display) -> RunCompletionError {
    tracing::error!(%error, "update-run completion processing failed");
    RunCompletionError::redacted("durable update result processing failed")
}

// Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
// redundant public crate visibility.
#[allow(clippy::redundant_pub_crate)]
async fn command_loop(
    consumer: async_nats::jetstream::consumer::PullConsumer,
    handler: NatsCommandHandler,
    control_handler: NatsControlHandler,
    concurrency: usize,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    let mut messages = consumer
        .messages()
        .await
        .map_err(|error| error.to_string())?;
    let permits = Arc::new(Semaphore::new(concurrency));
    let mut commands = JoinSet::new();
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            delivery = messages.next() => {
                let Some(delivery) = delivery else {
                    return Err(String::from("run command stream ended"));
                };
                let message = delivery.map_err(|error| error.to_string())?;
                let permit = Arc::clone(&permits)
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let handler = handler.clone();
                let control_handler = control_handler.clone();
                commands.spawn(async move {
                    let _permit = permit;
                    if message.message.subject.as_str() == CONTROL_EXECUTE_SUBJECT {
                        if let Err(error) = control_handler.handle(&message).await {
                            tracing::warn!(%error, "control command was not acknowledged");
                        }
                    } else if let Err(error) = handler.handle(&message).await {
                        tracing::warn!(%error, "run command was not acknowledged");
                    }
                });
            }
            result = commands.join_next(), if !commands.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "run command task panicked");
                }
            }
        }
    }
    while let Some(result) = commands.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "run command task panicked while draining");
        }
    }
    Ok(())
}

/// Running daemon handle.
pub struct RunningHephaestus {
    http_addr: SocketAddr,
    cancellation: CancellationToken,
    tasks: Vec<JoinHandle<Result<(), String>>>,
    pool: PgPool,
    nats_client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
    forge: Arc<PgForgeRepository>,
    run_repository: Arc<PgRunRepository>,
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
        if let Err(error) = self.flush_outbox().await {
            first_error.get_or_insert(error);
        }
        if let Err(error) = self.nats_client.drain().await {
            first_error.get_or_insert_with(|| AppError::Shutdown(error.to_string()));
        }
        self.pool.close().await;
        first_error.map_or(Ok(()), Err)
    }

    async fn flush_outbox(&self) -> Result<(), AppError> {
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
        for _pass in 0..100 {
            let forge = forge_publisher
                .publish_pending(self.forge.as_ref(), self.outbox_batch_size)
                .await
                .map_err(component("final forge outbox flush"))?;
            let releases = release_publisher
                .publish_pending(self.outbox_batch_size)
                .await
                .map_err(component("final release outbox flush"))?;
            let reviews = review_publisher
                .publish_pending(self.outbox_batch_size)
                .await
                .map_err(component("final review outbox flush"))?;
            let events = event_publisher
                .publish_pending(self.outbox_batch_size)
                .await
                .map_err(component("final product-event outbox flush"))?;
            if forge == 0 && releases == 0 && reviews == 0 && events == 0 {
                return Ok(());
            }
        }
        Err(AppError::Shutdown(String::from(
            "final outbox flush did not quiesce",
        )))
    }
}

/// Persisted run lifecycle event used by operational waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEventKind {
    /// The VM reached its running state.
    Running,
    /// The trusted host completed controlled result publication.
    ResultCompleted,
}

impl RunEventKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "run.running",
            Self::ResultCompleted => "result.completed",
        }
    }
}

struct PgAgentVmSpecFactory {
    pool: PgPool,
    root_images: BTreeMap<String, RootFilesystem>,
    runtime_policy: RuntimePolicy,
}

#[derive(Deserialize)]
struct StoredRuntimeContract {
    #[serde(alias = "executable")]
    command: String,
    arguments: Vec<String>,
    working_directory: String,
    root_image_digest: String,
}

#[derive(Deserialize)]
struct StoredEffectivePolicy {
    vcpus: u8,
    memory_mib: u32,
    network: StoredNetworkAccess,
}

#[derive(Deserialize)]
struct StoredUpdateHook {
    command: String,
    arguments: Vec<String>,
    timeout_seconds: u32,
    resources: StoredHookResources,
}

#[derive(Deserialize)]
struct StoredHookResources {
    vcpus: u8,
    memory_mib: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredNetworkAccess {
    Disabled,
    BrokerOnly,
    Egress,
}

#[async_trait]
impl VmSpecFactory for PgAgentVmSpecFactory {
    // Loading and validating the entire immutable launch contract in one
    // place keeps the no-substitution boundary directly auditable.
    #[allow(clippy::too_many_lines)]
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let stored = load_vm_launch_contract(&self.pool, run.id.as_uuid())
            .await
            .map_err(vm_factory_error)?
            .ok_or_else(|| invalid_spec("run", "exact reusable run provenance is missing"))?;
        if stored.release_state != "published" {
            return Err(invalid_spec(
                "release",
                "release is not currently available",
            ));
        }
        if !stored.revision_runnable || !stored.attachment_runnable {
            return Err(invalid_spec(
                "instance_revision",
                "the exact revision or attachment is not runnable",
            ));
        }
        if stored.requires_state != run.requires_state {
            return Err(invalid_spec(
                "requires_state",
                "run state capability does not match the immutable release agent",
            ));
        }
        let contract: StoredRuntimeContract =
            serde_json::from_value(stored.runtime_contract).map_err(vm_factory_error)?;
        let policy: StoredEffectivePolicy =
            serde_json::from_value(stored.effective_runtime_policy).map_err(vm_factory_error)?;
        let root = self
            .root_images
            .get(&contract.root_image_digest)
            .cloned()
            .ok_or_else(|| invalid_spec("root_image.reference", "root image is not configured"))?;
        let network_access = policy.network;
        let network = match network_access {
            StoredNetworkAccess::Disabled => NetworkMode::Disabled,
            StoredNetworkAccess::Egress => NetworkMode::UserMode {
                ingress: Vec::new(),
            },
            StoredNetworkAccess::BrokerOnly => NetworkMode::BrokerOnly,
        };
        let (program, arguments, working_directory, resources, timeout_seconds) = match run.kind {
            run_domain::RunKind::Normal => (
                format!("/release/{}", contract.command),
                contract.arguments,
                format!("/release/{}", contract.working_directory),
                VmResources {
                    vcpus: policy.vcpus,
                    memory_mib: policy.memory_mib,
                },
                None,
            ),
            run_domain::RunKind::Update => {
                let hook: StoredUpdateHook = serde_json::from_value(
                    stored
                        .update_hook
                        .ok_or_else(|| invalid_spec("update_hook", "update hook is missing"))?,
                )
                .map_err(vm_factory_error)?;
                (
                    format!("/release/{}", hook.command),
                    hook.arguments,
                    String::from("/release"),
                    VmResources {
                        vcpus: hook.resources.vcpus,
                        memory_mib: hook.resources.memory_mib,
                    },
                    Some(hook.timeout_seconds),
                )
            }
        };
        validate_runtime_policy(&self.runtime_policy, &resources, network_access)?;
        let mut labels = BTreeMap::from([
            (
                String::from("hephaestus.instance"),
                run.instance_id.to_string(),
            ),
            (
                String::from("hephaestus.instance-revision"),
                run.instance_revision_id.to_string(),
            ),
            (
                String::from("hephaestus.release"),
                run.release_id.to_string(),
            ),
            (
                String::from("hephaestus.platform-policy"),
                self.runtime_policy.version.clone(),
            ),
        ]);
        if let Some(seconds) = timeout_seconds {
            labels.insert(
                String::from("hephaestus.wall-clock-timeout-seconds"),
                seconds.to_string(),
            );
        }
        let env = guest_environment(run.kind, stored.agent_update_id)?;
        Ok(VmSpec {
            id: vm_trait::VmId(run.id.to_string()),
            root,
            disks: Vec::new(),
            mounts: Vec::<VmMount>::new(),
            resources,
            network,
            command: GuestCommand {
                program,
                args: arguments,
                env,
                working_dir: Some(working_directory.into()),
            },
            labels,
        })
    }
}

fn guest_environment(
    kind: run_domain::RunKind,
    update_id: Option<Uuid>,
) -> Result<BTreeMap<String, String>, VmError> {
    match (kind, update_id) {
        (run_domain::RunKind::Normal, _) => Ok(BTreeMap::new()),
        (run_domain::RunKind::Update, Some(update_id)) => Ok(BTreeMap::from([(
            String::from("HEPHAESTUS_UPDATE_ID"),
            update_id.to_string(),
        )])),
        (run_domain::RunKind::Update, None) => Err(invalid_spec(
            "update",
            "stable update identity is missing from the hook run",
        )),
    }
}

fn validate_runtime_policy(
    current: &RuntimePolicy,
    resources: &VmResources,
    network: StoredNetworkAccess,
) -> Result<(), VmError> {
    if resources.vcpus > current.max_vcpus || resources.memory_mib > current.max_memory_mib {
        return Err(invalid_spec(
            "effective_runtime_policy.resources",
            "the immutable resource selection exceeds the current platform policy",
        ));
    }
    let network_allowed = match network {
        StoredNetworkAccess::Disabled => true,
        StoredNetworkAccess::BrokerOnly => current.allow_broker_only,
        StoredNetworkAccess::Egress => current.allow_egress,
    };
    if !network_allowed {
        return Err(invalid_spec(
            "effective_runtime_policy.network",
            "the immutable network selection is no longer allowed by platform policy",
        ));
    }
    Ok(())
}

fn invalid_spec(field: &str, reason: &str) -> VmError {
    VmError::InvalidSpec {
        field: field.to_owned(),
        reason: reason.to_owned(),
    }
}

fn vm_factory_error(error: impl std::error::Error + Send + Sync + 'static) -> VmError {
    VmError::Provider {
        provider: String::from("hephaestus-app"),
        code: String::from("spec-factory"),
        source: Box::new(error),
    }
}

struct ResultFixtureProvider;

#[async_trait]
impl VmProvider for ResultFixtureProvider {
    fn name(&self) -> &'static str {
        "local-result-fixture"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(ResultFixtureInstance::new(spec)?))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct ResultFixtureInstance {
    id: VmId,
    work: Option<PathBuf>,
    output: Option<PathBuf>,
    exit_code: i32,
    uncertain_exit: bool,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
}

impl ResultFixtureInstance {
    fn new(spec: VmSpec) -> Result<Self, VmError> {
        let source = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-source");
        let work = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-work");
        let work = match (source, work) {
            (Some(source), Some(work)) => {
                if !source.read_only {
                    return Err(invalid_spec(
                        "mounts",
                        "repository source mount must be read-only",
                    ));
                }
                if work.read_only {
                    return Err(invalid_spec(
                        "mounts",
                        "repository work mount must be writable",
                    ));
                }
                Some(work.host_path.clone())
            }
            (None, None) => None,
            _ => {
                return Err(invalid_spec(
                    "mounts",
                    "repository source and work mounts must be paired",
                ));
            }
        };
        let output = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "build-output")
            .map(|mount| mount.host_path.clone());
        let exit_code = if spec.command.args.iter().any(|value| value == "fail") {
            23
        } else {
            0
        };
        let uncertain_exit = spec.command.args.iter().any(|value| value == "uncertain");
        let (events, _) = broadcast::channel(16);
        let (exit, _) = watch::channel(None);
        Ok(Self {
            id: spec.id,
            work,
            output,
            exit_code,
            uncertain_exit,
            events,
            exit,
        })
    }
}

#[async_trait]
impl VmInstance for ResultFixtureInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        drop(self.events.send(VmEvent::Started {
            ingress: Vec::new(),
        }));
        drop(self.events.send(VmEvent::Ready));
        if let Some(work) = &self.work {
            tokio::fs::write(
                work.join("input.txt"),
                "agent reviewed and changed this file\n",
            )
            .await
            .map_err(fixture_vm_error)?;
            let reports = work.join("reports");
            tokio::fs::create_dir_all(&reports)
                .await
                .map_err(fixture_vm_error)?;
            tokio::fs::write(reports.join("result.txt"), "durable browser E2E report\n")
                .await
                .map_err(fixture_vm_error)?;
        }
        if let Some(output) = &self.output {
            let reports = output.join("reports");
            tokio::fs::create_dir_all(&reports)
                .await
                .map_err(fixture_vm_error)?;
            tokio::fs::write(reports.join("result.txt"), "built browser artifact\n")
                .await
                .map_err(fixture_vm_error)?;
        }
        drop(self.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"fixture agent completed workspace edits\n".to_vec(),
        }));
        drop(self.events.send(VmEvent::Metric(VmMetric {
            name: String::from("fixture.cpu_ms"),
            value: 42.0,
            labels: BTreeMap::from([(String::from("phase"), String::from("result"))]),
        })));
        if self.work.is_some() {
            drop(self.events.send(VmEvent::FinalizeResult {
                message: String::from("fixture agent result"),
            }));
        }
        let exit = VmExit {
            code: (!self.uncertain_exit).then_some(self.exit_code),
            signal: self.uncertain_exit.then_some(9),
        };
        drop(self.events.send(VmEvent::Exited(exit.clone())));
        self.exit.send_replace(Some(exit));
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let current_exit = receiver.borrow_and_update().clone();
            if let Some(exit) = current_exit {
                return Ok(exit);
            }
            receiver
                .changed()
                .await
                .map_err(|_| VmError::InvalidState("fixture guest exited without a result"))?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        Ok(())
    }
}

fn fixture_vm_error(error: std::io::Error) -> VmError {
    VmError::Provider {
        provider: String::from("local-result-fixture"),
        code: String::from("workspace-write"),
        source: Box::new(error),
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
mod tests {
    use super::{
        BuildExecutionError, RuntimePolicy, StoredNetworkAccess,
        build_delivery_requires_redelivery, guest_environment, validate_runtime_policy,
    };
    use run_domain::RunKind;
    use uuid::Uuid;
    use vm_trait::{VmError, VmResources};

    fn policy() -> RuntimePolicy {
        RuntimePolicy {
            version: String::from("test/v2"),
            max_vcpus: 2,
            max_memory_mib: 1_024,
            allow_broker_only: true,
            allow_egress: false,
        }
    }

    #[test]
    fn current_platform_policy_accepts_an_unchanged_allowed_contract() {
        validate_runtime_policy(
            &policy(),
            &VmResources {
                vcpus: 2,
                memory_mib: 1_024,
            },
            StoredNetworkAccess::BrokerOnly,
        )
        .expect("contract remains allowed");
    }

    #[test]
    fn claimed_build_delivery_is_acknowledged_instead_of_poison_redelivered() {
        assert!(!build_delivery_requires_redelivery(
            &BuildExecutionError::AlreadyClaimed
        ));
        assert!(!build_delivery_requires_redelivery(
            &BuildExecutionError::GuestFailed
        ));
        assert!(build_delivery_requires_redelivery(
            &BuildExecutionError::Release
        ));
    }

    #[test]
    fn current_platform_policy_rejects_stored_resources_over_the_new_ceiling() {
        let error = validate_runtime_policy(
            &policy(),
            &VmResources {
                vcpus: 3,
                memory_mib: 1_024,
            },
            StoredNetworkAccess::Disabled,
        )
        .expect_err("contract exceeds the current ceiling");

        assert!(matches!(
            error,
            VmError::InvalidSpec { ref field, .. }
                if field == "effective_runtime_policy.resources"
        ));
    }

    #[test]
    fn current_platform_policy_rejects_network_access_disabled_since_resolution() {
        let error = validate_runtime_policy(
            &policy(),
            &VmResources {
                vcpus: 1,
                memory_mib: 512,
            },
            StoredNetworkAccess::Egress,
        )
        .expect_err("egress is no longer allowed");

        assert!(matches!(
            error,
            VmError::InvalidSpec { ref field, .. }
                if field == "effective_runtime_policy.network"
        ));
    }

    #[test]
    fn update_guest_receives_the_exact_stable_update_identity() {
        let update_id = Uuid::new_v4();
        let expected = update_id.to_string();
        let environment =
            guest_environment(RunKind::Update, Some(update_id)).expect("update environment");

        assert_eq!(
            environment.get("HEPHAESTUS_UPDATE_ID").map(String::as_str),
            Some(expected.as_str())
        );
        assert!(
            guest_environment(RunKind::Update, None).is_err(),
            "an update hook must fail closed when its durable identity is absent"
        );
        assert!(
            guest_environment(RunKind::Normal, Some(update_id))
                .expect("normal environment")
                .is_empty(),
            "normal agents must not receive an unrelated update identity"
        );
    }
}

//! Production composition root and supervised daemon lifecycle.

use agent_config::{AgentConfig, NetworkProfile};
use async_trait::async_trait;
use axum::{Router, routing::get};
use forge_service::{
    ForgeNatsOutboxPublisher, GitStorage, PgForgeRepository, ensure_forge_jetstream_topology,
};
use futures_util::StreamExt;
use git_http::{
    GitHttpLimits, GitHttpService, PostgresGitAuthorizer, PostgresOidcGitAuthenticator,
};
use identity_oidc::OidcVerifier;
use jsonwebtoken::{Algorithm, DecodingKey};
use review_domain::CONTROL_EXECUTE_SUBJECT;
use review_service::{NatsControlHandler, ReviewControlService, ReviewOutboxPublisher};
use run_domain::{CancelRun, Run};
use run_orchestrator::{
    NatsCommandHandler, NatsOutboxPublisher, PgRunRepository, RunOrchestrator, RunRepository,
    VmSpecFactory, ensure_jetstream_topology,
};
use runtime_types::{CommandId, RunId};
use sqlx::{PgPool, postgres::PgPoolOptions};
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
use vm_fake::FakeProvider;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId,
    VmInstance, VmMetric, VmMount, VmProvider, VmResources, VmSpec,
};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use workspace_local::{LocalWorkspaceConfig, LocalWorkspaceManager};

/// Ordered database migration expected by this application version.
pub const EXPECTED_DATABASE_MIGRATION: i64 = 4;

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

/// Complete configuration consumed by the composition root.
pub struct AppConfig {
    /// Runtime `PostgreSQL` connection string.
    pub database_url: String,
    /// NATS server connection string.
    pub nats_url: String,
    /// HTTP address for the API and Git transport.
    pub http_listen: SocketAddr,
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
    /// VM implementation selected for this process.
    pub vm_backend: VmBackendConfig,
    /// Immutable image references resolved to provider-neutral roots.
    pub root_images: BTreeMap<String, RootFilesystem>,
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
        if self.root_images.is_empty() {
            return Err(AppError::Configuration(String::from(
                "at least one root image mapping is required",
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
    git_authenticator: Arc<PostgresOidcGitAuthenticator>,
    git_authorizer: Arc<PostgresGitAuthorizer>,
    git_backend: PathBuf,
    git_limits: GitHttpLimits,
    http_listen: SocketAddr,
    run_repository: Arc<PgRunRepository>,
    review_control: ReviewControlService,
    orchestrator: Arc<RunOrchestrator>,
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
    pub async fn build(config: AppConfig) -> Result<Self, AppError> {
        config.validate()?;
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(&config.database_url)
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
        let review_control = ReviewControlService::new(pool.clone(), Arc::clone(&storage));
        let volumes = Arc::new(
            LocalVolumeStore::new(pool.clone(), config.volumes)
                .map_err(component("volume configuration"))?,
        );
        volumes
            .initialize()
            .await
            .map_err(component("volume initialization"))?;
        let mut workspaces = LocalWorkspaceManager::new(pool.clone(), config.workspaces)
            .map_err(component("workspace configuration"))?;
        workspaces
            .initialize()
            .map_err(component("workspace initialization"))?;
        let workspaces = Arc::new(workspaces);

        let provider: Arc<dyn VmProvider> = match config.vm_backend {
            VmBackendConfig::Fake => Arc::new(FakeProvider::new()),
            VmBackendConfig::FixtureResult => Arc::new(ResultFixtureProvider),
            VmBackendConfig::Custom(provider) => provider,
            VmBackendConfig::Libkrun(provider) => {
                Arc::new(LibkrunProvider::new(*provider).map_err(component("libkrun provider"))?)
            }
        };
        let spec_factory = Arc::new(PgAgentVmSpecFactory {
            pool: pool.clone(),
            root_images: config.root_images,
        });
        let orchestrator = Arc::new(
            RunOrchestrator::new(
                run_repository.clone(),
                volumes,
                provider,
                spec_factory,
                config.agent_state_capacity_bytes,
            )
            .with_workspace_manager(workspaces),
        );

        let verifier = Arc::new(OidcVerifier::new(
            config.oidc.issuer,
            &config.oidc.audience,
            config.oidc.algorithm,
            config.oidc.decoding_key,
        ));
        let git_authenticator = Arc::new(PostgresOidcGitAuthenticator::new(pool.clone(), verifier));
        let git_authorizer = Arc::new(PostgresGitAuthorizer::new(pool.clone()));
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
            git_authenticator,
            git_authorizer,
            git_backend: config.git_http_backend,
            git_limits: config.git_http_limits,
            http_listen: config.http_listen,
            run_repository,
            review_control,
            orchestrator,
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
        self.orchestrator
            .recover_after_restart()
            .await
            .map_err(component("run recovery"))?;
        ensure_forge_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("forge JetStream topology"))?;
        let consumer = ensure_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("run JetStream topology"))?;
        let git = GitHttpService::new(
            Arc::clone(&self.forge),
            Arc::clone(&self.storage),
            self.git_authenticator,
            self.git_authorizer,
            self.git_backend,
            self.git_limits,
        )
        .map_err(component("Git HTTP configuration"))?;
        let router = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .merge(git.router());
        let listener = tokio::net::TcpListener::bind(self.http_listen)
            .await
            .map_err(component("HTTP listener"))?;
        let http_addr = listener
            .local_addr()
            .map_err(component("HTTP listener address"))?;

        let cancellation = CancellationToken::new();
        let mut tasks = Vec::with_capacity(3);
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
        let run_repository_trait: Arc<dyn RunRepository> = self.run_repository.clone();
        let outbox = OutboxWorker {
            forge_publisher: ForgeNatsOutboxPublisher::new(self.jetstream.clone()),
            run_publisher: NatsOutboxPublisher::new(self.jetstream.clone()),
            review_publisher: ReviewOutboxPublisher::new(self.jetstream.clone(), self.pool.clone()),
            forge: Arc::clone(&self.forge),
            run_repository: run_repository_trait,
            poll_interval: self.outbox_poll_interval,
            batch_size: self.outbox_batch_size,
        };
        tasks.push(tokio::spawn(async move {
            outbox.run(publisher_cancel, publisher_ready_tx).await;
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
            http_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("HTTP task exited")))?;
            publisher_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("outbox task exited")))?;
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
            orchestrator: self.orchestrator,
            outbox_batch_size: self.outbox_batch_size,
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
    run_publisher: NatsOutboxPublisher,
    review_publisher: ReviewOutboxPublisher,
    forge: Arc<PgForgeRepository>,
    run_repository: Arc<dyn RunRepository>,
    poll_interval: Duration,
    batch_size: i64,
}

impl OutboxWorker {
    // Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
    // redundant public crate visibility.
    #[allow(clippy::redundant_pub_crate)]
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
                        .publish_pending(&self.forge, self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "forge outbox publication pass failed");
                    }
                    if let Err(error) = self
                        .run_publisher
                        .publish_pending(&self.run_repository, self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "run outbox publication pass failed");
                    }
                    if let Err(error) = self
                        .review_publisher
                        .publish_pending(self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "review outbox publication pass failed");
                    }
                }
            }
        }
    }
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
    orchestrator: Arc<RunOrchestrator>,
    outbox_batch_size: i64,
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
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM run_events
                    WHERE run_id = $1 AND event_type = $2
                 )",
            )
            .bind(run_id.as_uuid())
            .bind(kind.as_str())
            .fetch_one(&self.pool)
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
        let run_publisher = NatsOutboxPublisher::new(self.jetstream.clone());
        let review_publisher =
            ReviewOutboxPublisher::new(self.jetstream.clone(), self.pool.clone());
        let run_repository: Arc<dyn RunRepository> = self.run_repository.clone();
        for _pass in 0..100 {
            let forge = forge_publisher
                .publish_pending(&self.forge, self.outbox_batch_size)
                .await
                .map_err(component("final forge outbox flush"))?;
            let runs = run_publisher
                .publish_pending(&run_repository, self.outbox_batch_size)
                .await
                .map_err(component("final run outbox flush"))?;
            let reviews = review_publisher
                .publish_pending(self.outbox_batch_size)
                .await
                .map_err(component("final review outbox flush"))?;
            if forge == 0 && runs == 0 && reviews == 0 {
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
}

#[async_trait]
impl VmSpecFactory for PgAgentVmSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let config: serde_json::Value = sqlx::query_scalar(
            "SELECT revision.config
             FROM run_requests request
             JOIN agent_config_revisions revision
               ON revision.id = request.config_revision_id
             WHERE request.command_id = $1
               AND revision.status = 'valid'",
        )
        .bind(run.command_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(vm_factory_error)?
        .ok_or_else(|| invalid_spec("agent_config", "validated run configuration is missing"))?;
        let config: AgentConfig = serde_json::from_value(config).map_err(vm_factory_error)?;
        if !config.state_volume.enabled {
            return Err(invalid_spec(
                "state_volume.enabled",
                "the orchestrator currently requires agent state",
            ));
        }
        let root = self
            .root_images
            .get(&config.root_image.reference)
            .cloned()
            .ok_or_else(|| invalid_spec("root_image.reference", "root image is not configured"))?;
        let network = match config.network.profile {
            NetworkProfile::Disabled => NetworkMode::Disabled,
            NetworkProfile::Egress => NetworkMode::UserMode {
                ingress: Vec::new(),
            },
        };
        Ok(VmSpec {
            id: vm_trait::VmId(run.id.to_string()),
            root,
            disks: Vec::new(),
            mounts: Vec::<VmMount>::new(),
            resources: VmResources {
                vcpus: config.resources.vcpus,
                memory_mib: config.resources.memory_mib,
            },
            network,
            command: GuestCommand {
                program: config.guest.command,
                args: config.guest.arguments,
                env: BTreeMap::new(),
                working_dir: Some(config.guest.working_directory.into()),
            },
            labels: BTreeMap::from([(String::from("hephaestus.agent"), config.agent.name)]),
        })
    }
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
    work: PathBuf,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
}

impl ResultFixtureInstance {
    fn new(spec: VmSpec) -> Result<Self, VmError> {
        let source = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-source")
            .ok_or_else(|| invalid_spec("mounts", "repository source mount is missing"))?;
        if !source.read_only {
            return Err(invalid_spec(
                "mounts",
                "repository source mount must be read-only",
            ));
        }
        let work = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-work")
            .ok_or_else(|| invalid_spec("mounts", "repository work mount is missing"))?;
        if work.read_only {
            return Err(invalid_spec(
                "mounts",
                "repository work mount must be writable",
            ));
        }
        let (events, _) = broadcast::channel(16);
        let (exit, _) = watch::channel(None);
        Ok(Self {
            id: spec.id,
            work: work.host_path.clone(),
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
        tokio::fs::write(
            self.work.join("input.txt"),
            "agent reviewed and changed this file\n",
        )
        .await
        .map_err(fixture_vm_error)?;
        let reports = self.work.join("reports");
        tokio::fs::create_dir_all(&reports)
            .await
            .map_err(fixture_vm_error)?;
        tokio::fs::write(reports.join("result.txt"), "durable browser E2E report\n")
            .await
            .map_err(fixture_vm_error)?;
        drop(self.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"fixture agent completed workspace edits\n".to_vec(),
        }));
        drop(self.events.send(VmEvent::Metric(VmMetric {
            name: String::from("fixture.cpu_ms"),
            value: 42.0,
            labels: BTreeMap::from([(String::from("phase"), String::from("result"))]),
        })));
        drop(self.events.send(VmEvent::FinalizeResult {
            message: String::from("fixture agent result"),
        }));
        let exit = VmExit {
            code: Some(0),
            signal: None,
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
    let migration: Option<i64> =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(pool)
            .await
            .map_err(component("migration version check"))?;
    if migration != Some(EXPECTED_DATABASE_MIGRATION) {
        return Err(AppError::Configuration(format!(
            "database migration is {migration:?}; expected {EXPECTED_DATABASE_MIGRATION}"
        )));
    }
    let melange: bool = sqlx::query_scalar(
        "SELECT to_regprocedure(
            'check_permission(text,text,text,text,text)'
         ) IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(component("Mélange dispatcher check"))?;
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

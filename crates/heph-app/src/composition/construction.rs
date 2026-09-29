#[path = "construction/gateway.rs"]
mod gateway;

use super::{
    AppConfig, AppError, BuildExecutor, BuildExecutorConfig, BuildPreparation,
    CompositeGitAuthenticator, CompositeRunCompletionObserver, MailboxDispatchStore,
    MailboxRunCompletion, MailboxRunResources, Mutex, OidcGitAuthenticator, OidcVerifier,
    PgAgentVmSpecFactory, PgBuildRepository, PgRunLaunchAuthorizer, RunCompletionObserver,
    RuntimeGitHttpAuthenticator, component, prepare_build,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use super::{
    BrokerExecutor, GatewayEdgeRuntime, GatewayServiceLogMaintenanceScheduler, GitAuthenticator,
    GitHttpLimits, GitStorage, LocalArtifactStore, LocalKeyProvider, OciBuilderWorkers,
    PgForgeRepository, PgPool, PgRunRepository, PostgresGitAuthorizer, PostgresIdentityStore,
    PostgresMailboxRepository, PostgresReviewRepository, RegistryConfig, ReleaseService,
    ReviewControlService, RunOrchestrator, SecretService, UpdateRunCompletion,
};

/// Constructed application whose external tasks have not started.
pub struct HephaestusApp {
    pub(super) pool: PgPool,
    pub(super) application_pool: PgPool,
    pub(super) service_log_pool: PgPool,
    pub(super) nats_client: async_nats::Client,
    pub(super) jetstream: async_nats::jetstream::Context,
    pub(super) forge: Arc<PgForgeRepository>,
    pub(super) storage: Arc<GitStorage>,
    pub(super) identity_store: Arc<PostgresIdentityStore>,
    pub(super) git_authenticator: Arc<dyn GitAuthenticator>,
    pub(super) git_authorizer: Arc<PostgresGitAuthorizer>,
    pub(super) git_backend: PathBuf,
    pub(super) git_pre_receive_hook: PathBuf,
    pub(super) git_limits: GitHttpLimits,
    pub(super) registry: RegistryConfig,
    pub(super) http_listen: SocketAddr,
    pub(super) run_repository: Arc<PgRunRepository>,
    pub(super) mailbox_repository: Arc<PostgresMailboxRepository>,
    pub(super) review_repository: Arc<PostgresReviewRepository>,
    pub(super) review_control: ReviewControlService,
    pub(super) orchestrator: Arc<RunOrchestrator>,
    pub(super) build_executor: Arc<BuildExecutor>,
    pub(super) oci_builder_workers: Option<Arc<OciBuilderWorkers>>,
    pub(super) artifact_store: LocalArtifactStore,
    pub(super) result_artifact_root: PathBuf,
    pub(super) release_service: Arc<ReleaseService>,
    pub(super) update_completion: Arc<UpdateRunCompletion>,
    pub(super) secret_service: Arc<SecretService<LocalKeyProvider>>,
    pub(super) rpc_mediator_signing_key: [u8; 32],
    pub(super) internal_platform_policy: release_domain::RuntimePolicy,
    pub(super) internal_platform_policy_version: String,
    pub(super) secret_broker_socket: PathBuf,
    pub(super) secret_broker_executor: Arc<dyn BrokerExecutor>,
    pub(super) service_log_maintenance: Arc<GatewayServiceLogMaintenanceScheduler>,
    pub(super) gateway_edge: Option<GatewayEdgeRuntime>,
    pub(super) worker_concurrency: usize,
    pub(super) outbox_poll_interval: Duration,
    pub(super) outbox_batch_size: i64,
    pub(super) startup_timeout: Duration,
    pub(super) shutdown_timeout: Duration,
    pub(super) runtime_git_socket_path: Option<PathBuf>,
}

// Keep dependency assembly in one auditable sequence; the source file remains bounded.
#[allow(clippy::too_many_lines)]
pub async fn build(config: AppConfig) -> Result<HephaestusApp, AppError> {
    let BuildPreparation {
        config,
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
    } = Box::pin(prepare_build(config)).await?;
    let gateway_edge = gateway::build(gateway::GatewayInputs {
        database_url: config.database_url.clone(),
        gateway_service_host_id,
        pool: pool.clone(),
        gateway_release_runtime,
        gateway_edge_config,
        gateway_secret_keys,
        gateway_handoff_root,
        gateway_handoff_key,
        gateway_root_images,
        service_log_store,
        provider: Arc::clone(&provider),
    })
    .await?;
    let release_authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
    let release_service = Arc::new(ReleaseService::new(
        pool.clone(),
        release_authorizer.clone(),
    ));
    let artifact_store = LocalArtifactStore::new(super::canonicalize_release_artifact_root(
        release_artifact_root,
    )?)
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
    let launch_authorizer = Arc::new(PgRunLaunchAuthorizer::new(pool.clone(), release_authorizer));
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

    Ok(crate::HephaestusApp {
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

//! Generated Connect service composition and application dependencies.

use super::{
    DEFAULT_DEADLINE_SECONDS, GLOBAL_MAX_MESSAGE_BYTES, GLOBAL_MAX_REQUEST_BYTES,
    MAX_DEADLINE_SECONDS, MediatorAuthenticator, MutationReceipts, STREAM_IDLE_SECONDS, artifact,
    build, event, gateway, identity, image_catalog, instance, organization, pat, project, release,
    repository, repository_browser, run, secret,
};
use connectrpc::{ConnectRpcService, DeadlinePolicy, Limits, Router};
use connectrpc_reflection::Reflector;
use control_plane_postgres::ControlPlanePool as PgPool;
use event_application::MutationReceiptReader;
use forge_postgres::PgForgeRepository;
use forge_service::GitStorage;
use identity_application::{BrowserSessionStore, IdempotentIdentityResolver};
use release_artifact_store::LocalArtifactStore;
use rpc_proto::connect::hephaestus::identity::v1::IdentityServiceExt;
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Application dependencies shared by the generated Connect services.
pub struct ApplicationDependencies {
    pool: PgPool,
    application_pool: PgPool,
    ui_browser_worker_pool: PgPool,
    forge: Arc<PgForgeRepository>,
    mutation_receipt_reader: Arc<dyn MutationReceiptReader>,
    identity_resolver: Arc<dyn IdempotentIdentityResolver>,
    browser_sessions: Arc<dyn BrowserSessionStore>,
    release_service: Arc<release_postgres::ReleaseService>,
}

impl ApplicationDependencies {
    // Keep the composition root's explicit pool and adapter boundaries visible.
    #[allow(clippy::too_many_arguments)]
    /// Creates the dependency bundle used by the RPC service graph.
    pub fn new(
        pool: PgPool,
        application_pool: PgPool,
        ui_browser_worker_pool: PgPool,
        forge: Arc<PgForgeRepository>,
        mutation_receipt_reader: Arc<dyn MutationReceiptReader>,
        identity_resolver: Arc<dyn IdempotentIdentityResolver>,
        browser_sessions: Arc<dyn BrowserSessionStore>,
        release_service: Arc<release_postgres::ReleaseService>,
    ) -> Self {
        Self {
            pool,
            application_pool,
            ui_browser_worker_pool,
            forge,
            mutation_receipt_reader,
            identity_resolver,
            browser_sessions,
            release_service,
        }
    }
}

/// Builds the generated Connect services and reflection endpoints.
// The generated service inventory is clearest as one auditable router composition.
#[allow(clippy::too_many_lines)]
pub fn service(
    applications: ApplicationDependencies,
    storage: Arc<GitStorage>,
    artifact_store: LocalArtifactStore,
    result_artifact_root: PathBuf,
    mediator_signing_key: &[u8],
    commands: crate::application::commands::InternalCommandState,
    event_wakeups: Arc<dyn crate::application::event::EventWakeupSource>,
) -> Result<ConnectRpcService, RpcInitializationError> {
    let cursor_key: [u8; 32] = mediator_signing_key.try_into().map_err(|_| {
        RpcInitializationError::Descriptor(String::from("invalid event cursor key"))
    })?;
    let ui_installations = Arc::clone(&applications.release_service);
    let ui_navigator = Arc::new(release_postgres::PgUiInstallationNavigator::new(
        applications.application_pool.clone(),
    ));
    let ui_browser = Arc::new(release_postgres::PgUiBrowserSessionStore::new(
        applications.ui_browser_worker_pool.clone(),
        applications.application_pool.clone(),
    ));
    let ui_request_audit: Arc<dyn release_service::UiRequestAuditSink> =
        Arc::new(release_postgres::PgUiRequestAuditRepository::new(
            applications.ui_browser_worker_pool.clone(),
        ));
    let mutation_receipts = MutationReceipts::new(applications.mutation_receipt_reader, cursor_key);
    let pool = &applications.pool;
    let identity = Arc::new(identity::IdentityRpc::new(
        Arc::clone(&applications.identity_resolver),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
        Arc::clone(&applications.browser_sessions),
    ));
    let organization = Arc::new(organization::OrganizationRpc::new(
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
    ));
    let instance = Arc::new(instance::InstanceRpc::new(
        pool.clone(),
        commands.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    ));
    let secret = Arc::new(secret::SecretRpc::new(
        pool.clone(),
        commands,
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    ));
    let router = IdentityServiceExt::register(identity, Router::new());
    let router = gateway::register(
        router,
        pool,
        &applications.application_pool,
        Arc::clone(&storage),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
        cursor_key,
    );
    let router = rpc_proto::connect::hephaestus::instance::v1::AgentInstanceServiceExt::register(
        instance, router,
    );
    let router = rpc_proto::connect::hephaestus::organization::v1::OrganizationServiceExt::register(
        organization,
        router,
    );
    let router =
        rpc_proto::connect::hephaestus::secret::v1::SecretServiceExt::register(secret, router);
    let router = image_catalog::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
    );
    let router = project::register(
        router,
        pool.clone(),
        Arc::clone(&applications.forge),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    );
    let router = repository::register(
        router,
        pool.clone(),
        Arc::clone(&applications.forge),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    );
    let router = repository_browser::register(
        router,
        pool.clone(),
        storage,
        MediatorAuthenticator::new(mediator_signing_key),
    );
    let router = pat::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    );
    let router = build::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
        Arc::clone(&event_wakeups),
        cursor_key,
    );
    let router = release::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
        Arc::clone(&event_wakeups),
        cursor_key,
        ui_installations,
        ui_navigator,
        ui_browser,
        ui_request_audit,
    );
    let router = artifact::register(
        router,
        pool.clone(),
        artifact_store,
        mediator_signing_key,
        MediatorAuthenticator::new(mediator_signing_key),
    );
    let router = run::register(
        router,
        pool.clone(),
        result_artifact_root,
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts,
    );
    let router = event::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
        event_wakeups,
        cursor_key,
    );
    let descriptor_pool = rpc_proto::descriptor_pool()
        .map_err(|error| RpcInitializationError::Descriptor(error.to_string()))?;
    let reflector = Reflector::from_descriptor_pool(Arc::new(descriptor_pool))?;
    let router = connectrpc_reflection::install(router, reflector);
    Ok(ConnectRpcService::new(router)
        .with_limits(
            Limits::default()
                .max_request_body_size(GLOBAL_MAX_REQUEST_BYTES)
                .max_message_size(GLOBAL_MAX_MESSAGE_BYTES),
        )
        .with_deadline_policy(
            DeadlinePolicy::new()
                .with_max(Duration::from_secs(MAX_DEADLINE_SECONDS))
                .with_default_timeout(Duration::from_secs(DEFAULT_DEADLINE_SECONDS))
                .with_enforce_on_streams(true)
                .with_inter_message_timeout(Duration::from_secs(STREAM_IDLE_SECONDS)),
        ))
}

/// Failure to construct the static RPC service graph.
#[derive(Debug, thiserror::Error)]
pub enum RpcInitializationError {
    /// The checked-in descriptor set could not be decoded.
    #[error("RPC descriptor set is invalid: {0}")]
    Descriptor(String),
    /// The reflection index could not be built.
    #[error("RPC reflection configuration is invalid")]
    Reflection(#[from] connectrpc_reflection::ReflectionError),
}

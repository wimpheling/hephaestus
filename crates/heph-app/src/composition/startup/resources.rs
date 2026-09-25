use super::super::{
    AppError, Arc, BrokerServer, BrowserSessionStore, Duration, GitHttpService, HephaestusApp,
    InternalRegistryTokens, PgRegistryStore, PostgresBrowserSessionStore,
    PostgresRegistryNotificationInbox, PostgresRegistryReconciliation,
    PostgresRegistryScopeAuthorizer, PrivateGatewayDispatcherState,
    RegistryNotificationHttpService, RegistryReconciler, RegistryTokenHttpService, SocketAddr,
    ZotHttpRegistry, application, component, ensure_build_consumer,
    ensure_forge_jetstream_topology, ensure_jetstream_topology, ensure_mailbox_jetstream_topology,
    ensure_release_jetstream_topology, event_adapter, get, registry_caller_authentication, rpc,
    runtime_git_listener, ui_listener,
};
use crate::runtime_git_listener::RuntimeGitListener;
use async_nats::jetstream::consumer::PullConsumer;
use axum::Router;
use identity_application::IdempotentIdentityResolver;
use tokio::net::TcpListener;

type RegistryReconcilerState = RegistryReconciler<
    PostgresRegistryReconciliation,
    PostgresRegistryReconciliation,
    ZotHttpRegistry<InternalRegistryTokens>,
>;

pub(super) struct StartupResources {
    pub(super) build_consumer: PullConsumer,
    pub(super) consumer: PullConsumer,
    pub(super) mailbox_consumer: PullConsumer,
    pub(super) runtime_git_listener: Option<RuntimeGitListener>,
    pub(super) runtime_git_router: Option<Router>,
    pub(super) ui_listener: Option<(TcpListener, Router)>,
    pub(super) broker: BrokerServer,
    pub(super) registry_reconciler: RegistryReconcilerState,
    pub(super) registry_reconciliation_adapter: PostgresRegistryReconciliation,
    pub(super) registry_reconciliation_lease: Duration,
    pub(super) registry_reconciliation_interval: Duration,
    pub(super) gateway_listener: Option<(TcpListener, PrivateGatewayDispatcherState)>,
    pub(super) router: Router,
    pub(super) listener: TcpListener,
    pub(super) http_addr: SocketAddr,
}

// Keep resource construction in one ordered phase so each failure is mapped
// to its component before any worker task is started.
#[allow(clippy::too_many_lines)]
pub(super) async fn prepare(app: &HephaestusApp) -> Result<StartupResources, AppError> {
    app.build_executor
        .recover_after_restart()
        .await
        .map_err(component("build recovery"))?;
    app.orchestrator
        .recover_after_restart()
        .await
        .map_err(component("run recovery"))?;
    ensure_forge_jetstream_topology(&app.jetstream)
        .await
        .map_err(component("forge JetStream topology"))?;
    let build_consumer = ensure_build_consumer(&app.jetstream)
        .await
        .map_err(component("build JetStream consumer"))?;
    ensure_release_jetstream_topology(&app.jetstream)
        .await
        .map_err(component("release JetStream topology"))?;
    event_adapter::ensure_topology(&app.jetstream)
        .await
        .map_err(component("product-event JetStream topology"))?;
    let consumer = ensure_jetstream_topology(&app.jetstream)
        .await
        .map_err(component("run JetStream topology"))?;
    let mailbox_consumer = ensure_mailbox_jetstream_topology(&app.jetstream)
        .await
        .map_err(component("mailbox JetStream topology"))?;
    let receive_hook = app.git_pre_receive_hook.clone();
    let git = Arc::new(
        GitHttpService::new(
            Arc::clone(&app.forge),
            Arc::clone(&app.storage),
            app.git_authenticator.clone(),
            app.git_authorizer.clone(),
            app.git_backend.clone(),
            app.git_limits.clone(),
        )
        .and_then(|service| service.with_runtime_receive_hook(receive_hook))
        .map_err(component("Git HTTP configuration"))?,
    );
    let runtime_git_listener = app
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
    let ui_listener = ui_listener::build_ui_listener(app, Arc::clone(&git)).await?;
    let broker = BrokerServer::bind(
        app.secret_broker_socket.clone(),
        Arc::clone(&app.secret_broker_executor),
    )
    .map_err(component("secret broker listener"))?;
    let command_state = application::commands::InternalCommandState::new(
        Arc::clone(&app.release_service),
        Arc::clone(&app.secret_service),
        app.internal_platform_policy.clone(),
        app.internal_platform_policy_version.clone(),
    );
    let browser_sessions: Arc<dyn BrowserSessionStore> = Arc::new(
        PostgresBrowserSessionStore::new(app.pool.clone(), app.application_pool.clone()),
    );
    let rpc = rpc::service(
        rpc::ApplicationDependencies::new(
            app.pool.clone(),
            app.application_pool.clone(),
            app.service_log_pool.clone(),
            Arc::clone(&app.forge),
            Arc::new(event_postgres::PostgresMutationReceiptReader::new(
                app.pool.clone(),
            )),
            Arc::clone(&app.identity_store) as Arc<dyn IdempotentIdentityResolver>,
            Arc::clone(&browser_sessions),
            Arc::clone(&app.release_service),
        ),
        Arc::clone(&app.storage),
        app.artifact_store.clone(),
        app.result_artifact_root.clone(),
        &app.rpc_mediator_signing_key,
        command_state,
        Arc::new(event_adapter::NatsEventWakeups::new(
            app.nats_client.clone(),
        )),
    )
    .map_err(component("Connect RPC configuration"))?;
    let ui_request_audit: Arc<dyn release_service::UiRequestAuditSink> = Arc::new(
        release_postgres::PgUiRequestAuditRepository::new(app.service_log_pool.clone()),
    );
    let registry_store = PgRegistryStore::new(app.pool.clone());
    let registry_reconciliation_adapter = PostgresRegistryReconciliation {
        store: registry_store.clone(),
    };
    let registry_reconciler = RegistryReconciler::new(
        registry_reconciliation_adapter.clone(),
        registry_reconciliation_adapter.clone(),
        ZotHttpRegistry::new(
            app.registry.zot.clone(),
            Arc::new(InternalRegistryTokens {
                issuer: Arc::clone(&app.registry.token_issuer),
            }),
        )
        .map_err(component("Zot reconciliation client"))?,
    );
    let registry_reconciliation_lease = app.registry.reconciliation_lease;
    let registry_reconciliation_interval = app.registry.reconciliation_interval;
    let registry_tokens = RegistryTokenHttpService::new(
        Arc::clone(&app.registry.token_issuer),
        Arc::new(PostgresRegistryScopeAuthorizer {
            store: registry_store.clone(),
        }),
    )
    .router()
    .layer(axum::middleware::from_fn_with_state(
        Arc::clone(&app.git_authenticator),
        registry_caller_authentication,
    ));
    let registry_notifications = RegistryNotificationHttpService::new(
        app.registry.notification_callback.clone(),
        Arc::new(PostgresRegistryNotificationInbox {
            store: registry_store.clone(),
        }),
    )
    .router();
    let gateway_listener = if let Some(gateway) = &app.gateway_edge {
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
                rpc::MediatorAuthenticator::new(&app.rpc_mediator_signing_key),
                browser_sessions,
            )
            .with_ui_request_audit_sink(ui_request_audit),
            rpc::mediator_identity_middleware,
        ));
    let listener = tokio::net::TcpListener::bind(app.http_listen)
        .await
        .map_err(component("HTTP listener"))?;
    let http_addr = listener
        .local_addr()
        .map_err(component("HTTP listener address"))?;

    if let Some(workers) = &app.oci_builder_workers {
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

    Ok(StartupResources {
        build_consumer,
        consumer,
        mailbox_consumer,
        runtime_git_listener,
        runtime_git_router,
        ui_listener,
        broker,
        registry_reconciler,
        registry_reconciliation_adapter,
        registry_reconciliation_lease,
        registry_reconciliation_interval,
        gateway_listener,
        router,
        listener,
        http_addr,
    })
}

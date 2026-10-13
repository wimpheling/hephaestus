#[path = "workers/gateway.rs"]
mod gateway;
#[path = "workers/listeners.rs"]
mod listeners;
use super::super::{
    AppError, Arc, CancellationToken, ForgeNatsOutboxPublisher, GatewayServiceBootRecovery,
    HephaestusApp, MailboxCommandHandler, MailboxDispatchStore, MailboxOutboxPublisher,
    NatsCommandHandler, NatsControlHandler, NatsMailboxCommandHandler, OutboxWorker,
    ReleaseOutboxPublisher, ReviewOutboxPublisher, RunningHephaestus, SocketAddr, build_loop,
    clone_service_supervisor_context, command_loop, component, event_adapter,
    gateway_reconciliation_loop_with_context, mailbox_command_loop, mailbox_recovery_loop,
    oci_builder_loop, registry_reconciliation_loop, secret_revocation_loop,
    update_admission_reconciliation_loop,
};
use super::readiness::{self, StartupTasks};
use super::resources::StartupResources;
use tokio::sync::oneshot;

pub(super) async fn launch(
    app: HephaestusApp,
    resources: StartupResources,
) -> Result<RunningHephaestus, AppError> {
    let parts = launch_parts(&app, resources, None).await?;
    Ok(super::running(
        app,
        parts.http_addr,
        parts.cancellation,
        parts.tasks,
    ))
}

pub struct StartedServices {
    pub http_addr: SocketAddr,
    pub cancellation: CancellationToken,
    pub tasks: Vec<tokio::task::JoinHandle<Result<(), String>>>,
}

// Keep the existing ordered worker/readiness sequence together.
#[allow(clippy::too_many_lines)]
pub(super) async fn launch_parts(
    app: &HephaestusApp,
    resources: StartupResources,
    managed: Option<Arc<crate::managed_backend::Services>>,
) -> Result<StartedServices, AppError> {
    let StartupResources {
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
        ..
    } = resources;
    let cancellation = CancellationToken::new();
    if let Some(services) = &managed {
        services.token(&cancellation)?;
    }
    let mut tasks = crate::managed_backend::OwnedTasks::new(managed.as_deref());
    let service_log_maintenance = Arc::clone(&app.service_log_maintenance);
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
    let update_reconcile_observer = Arc::clone(&app.update_completion);
    let update_reconcile_interval = app.outbox_poll_interval;
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
    gateway::spawn(app, &cancellation, &mut tasks)?;
    let listeners::Ready {
        broker: broker_ready_rx,
        gateway: gateway_ready_rx,
        ui: ui_ready_rx,
        runtime_git: runtime_git_ready_rx,
        http: http_ready_rx,
    } = listeners::spawn(
        listeners::Inputs {
            broker,
            gateway_listener,
            ui_listener,
            runtime_git_listener,
            runtime_git_router,
            listener,
            router,
        },
        &cancellation,
        &mut tasks,
    );
    let (publisher_ready_tx, publisher_ready_rx) = oneshot::channel();
    let publisher_cancel = cancellation.clone();
    let outbox = OutboxWorker {
        forge_publisher: ForgeNatsOutboxPublisher::new(app.jetstream.clone()),
        release_publisher: ReleaseOutboxPublisher::new(app.jetstream.clone(), app.pool.clone()),
        review_publisher: ReviewOutboxPublisher::new(
            app.jetstream.clone(),
            Arc::clone(&app.review_repository) as Arc<dyn review_service::ReviewOutboxStore>,
        ),
        event_publisher: event_adapter::EventPublisher::new(
            app.jetstream.clone(),
            Arc::new(event_postgres::PostgresProductEventOutbox::new(
                app.pool.clone(),
            )),
            app.rpc_mediator_signing_key,
        ),
        mailbox_publisher: MailboxOutboxPublisher::new(
            app.jetstream.clone(),
            app.mailbox_repository.clone(),
        ),
        forge: Arc::clone(&app.forge),
        poll_interval: app.outbox_poll_interval,
        batch_size: app.outbox_batch_size,
    };
    tasks.push(tokio::spawn(async move {
        outbox.run(publisher_cancel, publisher_ready_tx).await;
        Ok(())
    }));
    let (mailbox_recovery_ready_tx, mailbox_recovery_ready_rx) = oneshot::channel();
    let mailbox_recovery_cancel = cancellation.clone();
    let mailbox_recovery_store: Arc<dyn MailboxDispatchStore> = app.mailbox_repository.clone();
    let mailbox_recovery_interval = app.outbox_poll_interval;
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
    let secret_reconcile_pool = app.pool.clone();
    let secret_reconcile_orchestrator = routed_executor(app)?;
    let secret_reconcile_interval = app.outbox_poll_interval;
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
    let mailbox_handler = NatsMailboxCommandHandler::new(MailboxCommandHandler::new_routed(
        app.mailbox_repository.clone(),
        routed_executor(app)?,
    ));
    let mailbox_concurrency = app.worker_concurrency;
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
    let build_executor = Arc::clone(&app.build_executor);
    let build_concurrency = app.worker_concurrency;
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

    if let Some(workers) = &app.oci_builder_workers {
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
    let handler = NatsCommandHandler::new_routed(routed_executor(app)?);
    let control_handler = NatsControlHandler::new(app.review_control.clone());
    let concurrency = app.worker_concurrency;
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

    let startup_tasks = StartupTasks {
        cancellation,
        tasks,
        broker_ready_rx,
        update_reconcile_ready_rx,
        http_ready_rx,
        gateway_ready_rx,
        ui_ready_rx,
        runtime_git_ready_rx,
        publisher_ready_rx,
        mailbox_recovery_ready_rx,
        secret_reconcile_ready_rx,
        build_ready_rx,
        consumer_ready_rx,
        mailbox_consumer_ready_rx,
    };
    let (cancellation, tasks) = readiness::wait(app.startup_timeout, startup_tasks).await?;

    Ok(StartedServices {
        http_addr,
        cancellation,
        tasks,
    })
}

fn routed_executor(
    app: &HephaestusApp,
) -> Result<Arc<dyn run_orchestrator::RunCommandExecutor>, AppError> {
    match &app.managed_backend {
        Some(lease) => Ok(lease.executor()?),
        None => Ok(Arc::clone(&app.orchestrator) as Arc<dyn run_orchestrator::RunCommandExecutor>),
    }
}

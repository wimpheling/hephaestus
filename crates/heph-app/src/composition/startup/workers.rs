use super::super::{
    AppError, Arc, CancellationToken, ForgeNatsOutboxPublisher, GatewayServiceBootRecovery,
    HephaestusApp, MailboxCommandHandler, MailboxDispatchStore, MailboxOutboxPublisher,
    NatsCommandHandler, NatsControlHandler, NatsMailboxCommandHandler, OutboxWorker,
    ReleaseOutboxPublisher, ReviewOutboxPublisher, Router, RunningHephaestus, SocketAddr, any,
    build_loop, clone_service_supervisor_context, command_loop, component, event_adapter,
    gateway_reconciliation_loop_with_context, mailbox_command_loop, mailbox_recovery_loop,
    oci_builder_loop, private_gateway_dispatch, registry_reconciliation_loop,
    secret_revocation_loop, spawn_runtime_git_listener, update_admission_reconciliation_loop,
};
use super::readiness::{self, StartupTasks};
use super::resources::StartupResources;
use tokio::sync::oneshot;

// Keep worker startup and its readiness channels together so cancellation and
// task ownership remain auditable at the startup boundary.
#[allow(clippy::too_many_lines)]
pub(super) async fn launch(
    mut app: HephaestusApp,
    resources: StartupResources,
) -> Result<RunningHephaestus, AppError> {
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
    let mut tasks = Vec::with_capacity(10);
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
    if let Some(gateway) = app.gateway_edge.take() {
        let gateway_reconcile_cancel = cancellation.clone();
        let gateway_authority = gateway.authority.clone();
        let gateway_recovery_authority = gateway.recovery_authority.clone();
        let service_supervisor_context =
            clone_service_supervisor_context(&gateway.service_supervisor_context);
        let service_boot_recovery = GatewayServiceBootRecovery::new(gateway.service_boot_context)
            .map_err(component("gateway service boot recovery"))?;
        let service_claim_resolution = Arc::clone(&gateway.service_claim_resolution);
        let service_expired_claim_recovery = Arc::clone(&gateway.service_expired_claim_recovery);
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
    let secret_reconcile_orchestrator = Arc::clone(&app.orchestrator);
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
    let mailbox_handler = NatsMailboxCommandHandler::new(MailboxCommandHandler::new(
        app.mailbox_repository.clone(),
        Arc::clone(&app.orchestrator),
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
    let handler = NatsCommandHandler::new(Arc::clone(&app.orchestrator));
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

    Ok(super::running(app, http_addr, cancellation, tasks))
}

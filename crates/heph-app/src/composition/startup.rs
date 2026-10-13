#[path = "startup/readiness.rs"]
mod readiness;
#[path = "startup/resources.rs"]
mod resources;
#[path = "startup/workers.rs"]
mod workers;

pub use workers::StartedServices;

use super::{AppError, HephaestusApp, RunningHephaestus};
use crate::running::OrdinaryCustody;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub async fn start(app: HephaestusApp) -> Result<RunningHephaestus, AppError> {
    if app.managed_backend.is_some() {
        return crate::managed_backend::start_owned(app)?.wait().await;
    }
    let resources = resources::prepare(&app).await?;
    workers::launch(app, resources).await
}

pub async fn managed_parts(
    app: &HephaestusApp,
    services: std::sync::Arc<crate::managed_backend::Services>,
) -> Result<StartedServices, AppError> {
    require_managed_startup(
        app.managed_backend
            .as_ref()
            .ok_or_else(|| AppError::Readiness("managed startup lost its original lease".into()))?,
    )?;
    let resources = resources::prepare(app).await?;
    app.managed_backend
        .as_ref()
        .ok_or_else(|| AppError::Readiness("managed startup lease missing".into()))?
        .require_recovered()?;
    workers::launch_parts(app, resources, Some(services)).await
}

pub fn running(
    app: HephaestusApp,
    http_addr: std::net::SocketAddr,
    cancellation: CancellationToken,
    tasks: Vec<JoinHandle<Result<(), String>>>,
) -> RunningHephaestus {
    RunningHephaestus {
        managed_backend: app.managed_backend,
        http_addr,
        cancellation,
        tasks,
        pool: app.pool,
        application_pool: app.application_pool,
        worker_pool: app.worker_pool,
        service_log_pool: app.service_log_pool,
        nats_client: app.nats_client,
        jetstream: app.jetstream,
        forge: app.forge,
        run_repository: app.run_repository,
        mailbox_repository: app.mailbox_repository,
        review_repository: app.review_repository,
        orchestrator: app.orchestrator,
        outbox_batch_size: app.outbox_batch_size,
        product_event_cursor_key: app.rpc_mediator_signing_key,
        shutdown_timeout: app.shutdown_timeout,
        ordinary: std::sync::Arc::new(OrdinaryCustody {
            _build: app.build_executor,
            _oci: app.oci_builder_workers,
            _gateway: app.gateway_edge,
            _original_app: None,
        }),
    }
}

/// Internal shared-original construction, without a public App clone API.
pub fn running_borrowed(
    app: std::sync::Arc<HephaestusApp>,
    parts: StartedServices,
) -> Result<RunningHephaestus, AppError> {
    let lease = app
        .managed_backend
        .as_ref()
        .ok_or_else(|| AppError::Readiness("original managed App lease missing".into()))?;
    Ok(RunningHephaestus {
        managed_backend: Some(lease.fork_service_lease()),
        http_addr: parts.http_addr,
        cancellation: parts.cancellation,
        tasks: parts.tasks,
        pool: app.pool.clone(),
        application_pool: app.application_pool.clone(),
        worker_pool: app.worker_pool.clone(),
        service_log_pool: app.service_log_pool.clone(),
        nats_client: app.nats_client.clone(),
        jetstream: app.jetstream.clone(),
        forge: std::sync::Arc::clone(&app.forge),
        run_repository: std::sync::Arc::clone(&app.run_repository),
        mailbox_repository: std::sync::Arc::clone(&app.mailbox_repository),
        review_repository: std::sync::Arc::clone(&app.review_repository),
        orchestrator: std::sync::Arc::clone(&app.orchestrator),
        outbox_batch_size: app.outbox_batch_size,
        product_event_cursor_key: app.rpc_mediator_signing_key,
        shutdown_timeout: app.shutdown_timeout,
        ordinary: std::sync::Arc::new(OrdinaryCustody {
            _build: std::sync::Arc::clone(&app.build_executor),
            _oci: app.oci_builder_workers.clone(),
            // The complete original gateway is retained within this same App.
            _gateway: None,
            _original_app: Some(app),
        }),
    })
}

fn require_managed_startup(lease: &crate::managed_backend::NodeLease) -> Result<(), AppError> {
    lease.require_recovered()?;
    lease.original_invocation()?.validate_owners()
}

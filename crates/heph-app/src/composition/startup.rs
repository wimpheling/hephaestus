#[path = "startup/readiness.rs"]
mod readiness;
#[path = "startup/resources.rs"]
mod resources;
#[path = "startup/workers.rs"]
mod workers;

use super::{AppError, HephaestusApp, RunningHephaestus};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub async fn start(app: HephaestusApp) -> Result<RunningHephaestus, AppError> {
    let resources = resources::prepare(&app).await?;
    workers::launch(app, resources).await
}

pub fn running(
    app: HephaestusApp,
    http_addr: std::net::SocketAddr,
    cancellation: CancellationToken,
    tasks: Vec<JoinHandle<Result<(), String>>>,
) -> RunningHephaestus {
    RunningHephaestus {
        http_addr,
        cancellation,
        tasks,
        pool: app.pool,
        application_pool: app.application_pool,
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
    }
}

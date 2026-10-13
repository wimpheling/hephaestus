//! Existing listener tasks, immediately transferred to the original service slot.
use crate::managed_backend::OwnedTasks;
use crate::runtime_git_listener::RuntimeGitListener;
use crate::{
    BrokerServer, PrivateGatewayDispatcherState, private_gateway_dispatch,
    spawn_runtime_git_listener,
};
use axum::{Router, routing::any};
use std::net::SocketAddr;
use tokio::{net::TcpListener, sync::oneshot};
use tokio_util::sync::CancellationToken;
pub(super) struct Inputs {
    pub(super) broker: BrokerServer,
    pub(super) gateway_listener: Option<(TcpListener, PrivateGatewayDispatcherState)>,
    pub(super) ui_listener: Option<(TcpListener, Router)>,
    pub(super) runtime_git_listener: Option<RuntimeGitListener>,
    pub(super) runtime_git_router: Option<Router>,
    pub(super) listener: TcpListener,
    pub(super) router: Router,
}
pub(super) struct Ready {
    pub(super) broker: oneshot::Receiver<()>,
    pub(super) gateway: Option<oneshot::Receiver<()>>,
    pub(super) ui: Option<oneshot::Receiver<()>>,
    pub(super) runtime_git: Option<oneshot::Receiver<()>>,
    pub(super) http: oneshot::Receiver<()>,
}
pub(super) fn spawn(
    inputs: Inputs,
    cancellation: &CancellationToken,
    tasks: &mut OwnedTasks,
) -> Ready {
    let Inputs {
        broker,
        gateway_listener,
        ui_listener,
        runtime_git_listener,
        runtime_git_router,
        listener,
        router,
    } = inputs;
    let broker_ready_rx = spawn_broker(broker, cancellation, tasks);
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
                spawn_runtime_git_listener(listener, router, cancellation, tasks)
            });
    tasks.capture();
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
    Ready {
        broker: broker_ready_rx,
        gateway: gateway_ready_rx,
        ui: ui_ready_rx,
        runtime_git: runtime_git_ready_rx,
        http: http_ready_rx,
    }
}

fn spawn_broker(
    broker: BrokerServer,
    cancellation: &CancellationToken,
    tasks: &mut OwnedTasks,
) -> oneshot::Receiver<()> {
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
    broker_ready_rx
}

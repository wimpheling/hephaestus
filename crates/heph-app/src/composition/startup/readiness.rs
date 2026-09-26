use super::super::{AppError, CancellationToken, Duration, JoinHandle, reap_failed_start};
use tokio::sync::oneshot;

pub(super) struct StartupTasks {
    pub(super) cancellation: CancellationToken,
    pub(super) tasks: Vec<JoinHandle<Result<(), String>>>,
    pub(super) broker_ready_rx: oneshot::Receiver<()>,
    pub(super) update_reconcile_ready_rx: oneshot::Receiver<()>,
    pub(super) http_ready_rx: oneshot::Receiver<()>,
    pub(super) gateway_ready_rx: Option<oneshot::Receiver<()>>,
    pub(super) ui_ready_rx: Option<oneshot::Receiver<()>>,
    pub(super) runtime_git_ready_rx: Option<oneshot::Receiver<()>>,
    pub(super) publisher_ready_rx: oneshot::Receiver<()>,
    pub(super) mailbox_recovery_ready_rx: oneshot::Receiver<()>,
    pub(super) secret_reconcile_ready_rx: oneshot::Receiver<()>,
    pub(super) build_ready_rx: oneshot::Receiver<()>,
    pub(super) consumer_ready_rx: oneshot::Receiver<()>,
    pub(super) mailbox_consumer_ready_rx: oneshot::Receiver<()>,
}

pub(super) async fn wait(
    startup_timeout: Duration,
    startup: StartupTasks,
) -> Result<(CancellationToken, Vec<JoinHandle<Result<(), String>>>), AppError> {
    let StartupTasks {
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
    } = startup;
    let readiness = async {
        broker_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("secret broker task exited")))?;
        update_reconcile_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("update reconciliation task exited")))?;
        http_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("HTTP task exited")))?;
        if let Some(gateway_ready_rx) = gateway_ready_rx {
            gateway_ready_rx.await.map_err(|_| {
                AppError::Readiness(String::from("gateway private dispatcher task exited"))
            })?;
        }
        if let Some(ui_ready_rx) = ui_ready_rx {
            ui_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("UI origin listener task exited")))?;
        }
        if let Some(runtime_git_ready_rx) = runtime_git_ready_rx {
            runtime_git_ready_rx.await.map_err(|_| {
                AppError::Readiness(String::from("runtime Git listener task exited"))
            })?;
        }
        publisher_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("outbox task exited")))?;
        mailbox_recovery_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("mailbox recovery task exited")))?;
        secret_reconcile_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("secret reconciliation task exited")))?;
        build_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("build task exited")))?;
        consumer_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("consumer task exited")))?;
        mailbox_consumer_ready_rx
            .await
            .map_err(|_| AppError::Readiness(String::from("mailbox consumer task exited")))?;
        Ok::<(), AppError>(())
    };
    match tokio::time::timeout(startup_timeout, readiness).await {
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
    Ok((cancellation, tasks))
}

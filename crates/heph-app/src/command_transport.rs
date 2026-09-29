use super::{
    Arc, BuildExecutionError, BuildExecutor, BuildRequestId, CancellationToken, Duration,
    NatsCommandHandler, NatsControlHandler, NatsMailboxCommandHandler, Semaphore,
};
use uuid::Uuid;

use forge_service::{
    BUILD_REQUESTED_SUBJECT, BUILD_RETRY_REQUESTED_SUBJECT, BUILD_VERIFY_REQUESTED_SUBJECT,
};
use futures_util::StreamExt;
use heph_run::RunCompletionError;
use review_domain::CONTROL_EXECUTE_SUBJECT;
use serde::Deserialize;
use tokio::{sync::oneshot, task::JoinSet};
#[derive(Deserialize)]
struct BuildRequestedPayload {
    build_request_id: Uuid,
}

pub async fn build_loop(
    consumer: async_nats::jetstream::consumer::PullConsumer,
    executor: Arc<BuildExecutor>,
    concurrency: usize,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    let mut messages = consumer
        .messages()
        .await
        .map_err(|error| error.to_string())?;
    let permits = Arc::new(Semaphore::new(concurrency));
    let mut builds = JoinSet::new();
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            delivery = messages.next() => {
                let Some(delivery) = delivery else {
                    return Err(String::from("build command stream ended"));
                };
                let message = delivery.map_err(|error| error.to_string())?;
                let permit = Arc::clone(&permits)
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let executor = Arc::clone(&executor);
                builds.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_build_message(&executor, &message).await {
                        tracing::warn!(%error, "build command handling failed");
                    }
                });
            }
            result = builds.join_next(), if !builds.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "build command task panicked");
                }
            }
        }
    }
    while let Some(result) = builds.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "build command task panicked while draining");
        }
    }
    Ok(())
}

async fn handle_build_message(
    executor: &BuildExecutor,
    message: &async_nats::jetstream::Message,
) -> Result<(), String> {
    let retry = message.message.subject.as_str() == BUILD_RETRY_REQUESTED_SUBJECT;
    let verify = message.message.subject.as_str() == BUILD_VERIFY_REQUESTED_SUBJECT;
    if message.message.subject.as_str() != BUILD_REQUESTED_SUBJECT && !retry && !verify {
        message
            .ack_with(async_nats::jetstream::AckKind::Term)
            .await
            .map_err(|error| error.to_string())?;
        return Err(String::from("unknown build command subject"));
    }
    let payload: BuildRequestedPayload = match serde_json::from_slice(&message.payload) {
        Ok(payload) => payload,
        Err(error) => {
            message
                .ack_with(async_nats::jetstream::AckKind::Term)
                .await
                .map_err(|ack_error| ack_error.to_string())?;
            return Err(error.to_string());
        }
    };
    let operation = async {
        if verify {
            executor
                .verify(BuildRequestId::from_uuid(payload.build_request_id))
                .await
        } else if retry {
            executor
                .retry(BuildRequestId::from_uuid(payload.build_request_id))
                .await
                .map(|_| ())
        } else {
            executor
                .execute(BuildRequestId::from_uuid(payload.build_request_id))
                .await
                .map(|_| ())
        }
    };
    tokio::pin!(operation);
    let mut progress = tokio::time::interval(Duration::from_secs(10));
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut operation => {
                match result {
                    Ok(()) => {
                        message.double_ack().await.map_err(|error| error.to_string())?;
                        return Ok(());
                    }
                    Err(error) if build_delivery_requires_redelivery(&error) => {
                        return Err(error.to_string());
                    }
                    Err(error) => {
                        message.double_ack().await.map_err(|ack_error| ack_error.to_string())?;
                        return Err(error.to_string());
                    }
                }
            }
            _ = progress.tick() => {
                if let Err(error) = message
                    .ack_with(async_nats::jetstream::AckKind::Progress)
                    .await
                {
                    tracing::warn!(%error, "failed to acknowledge build progress");
                }
            }
        }
    }
}

pub const fn build_delivery_requires_redelivery(error: &BuildExecutionError) -> bool {
    matches!(
        error,
        BuildExecutionError::Database(_)
            | BuildExecutionError::Release
            | BuildExecutionError::ImageUnavailable
            | BuildExecutionError::VmCleanup
    )
}

pub fn completion_error(error: impl std::fmt::Display) -> RunCompletionError {
    tracing::error!(%error, "update-run completion processing failed");
    RunCompletionError::redacted("durable update result processing failed")
}

// Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
// redundant public crate visibility.
#[allow(clippy::redundant_pub_crate)]
pub async fn mailbox_command_loop(
    consumer: async_nats::jetstream::consumer::PullConsumer,
    handler: NatsMailboxCommandHandler,
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
                    return Err(String::from("mailbox command stream ended"));
                };
                let message = delivery.map_err(|error| error.to_string())?;
                let permit = Arc::clone(&permits)
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let handler = handler.clone();
                commands.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handler.handle(&message).await {
                        tracing::warn!(%error, "mailbox command was not acknowledged");
                    }
                });
            }
            result = commands.join_next(), if !commands.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "mailbox command task panicked");
                }
            }
        }
    }
    while let Some(result) = commands.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "mailbox command task panicked while draining");
        }
    }
    Ok(())
}

// Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
// redundant public crate visibility.
#[allow(clippy::redundant_pub_crate)]
pub async fn command_loop(
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

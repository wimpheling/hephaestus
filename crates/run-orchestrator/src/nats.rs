use async_nats::{HeaderMap, jetstream};
use futures_util::StreamExt;
use run_domain::{CancelRun, StartRun};
use std::sync::Arc;

use crate::{OrchestratorError, RepositoryError, RunOrchestrator, RunRepository};

const ACK_PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_CONCURRENT_COMMANDS: usize = 64;
/// Durable subject carrying `StartRun` commands.
pub const START_RUN_SUBJECT: &str = "heph.run.command.start.v1";
/// Forge-originated durable subject carrying `StartRun` commands.
pub const FORGE_START_RUN_SUBJECT: &str = "hephaestus.run.start";
/// Durable subject carrying `CancelRun` commands.
pub const CANCEL_RUN_SUBJECT: &str = "heph.run.command.cancel.v1";
/// Durable subject carrying run lifecycle events.
pub const LIFECYCLE_EVENT_SUBJECT: &str = "heph.run.event.lifecycle.v1";
const COMMAND_STREAM: &str = "HEPH_RUN_COMMANDS";
const EVENT_STREAM: &str = "HEPH_RUN_EVENTS";
const COMMAND_CONSUMER: &str = "run-orchestrator-v1";

/// Creates or resolves the durable command/event streams and command
/// consumer.
///
/// # Errors
///
/// Returns an error when the `JetStream` account rejects topology creation.
pub async fn ensure_jetstream_topology(
    context: &jetstream::Context,
) -> Result<jetstream::consumer::PullConsumer, TopologyError> {
    use jetstream::stream::{Config, RetentionPolicy, StorageType};

    let commands = context
        .get_or_create_stream(Config {
            name: COMMAND_STREAM.to_owned(),
            subjects: vec![
                START_RUN_SUBJECT.to_owned(),
                FORGE_START_RUN_SUBJECT.to_owned(),
                CANCEL_RUN_SUBJECT.to_owned(),
            ],
            retention: RetentionPolicy::WorkQueue,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|error| TopologyError(error.to_string()))?;
    context
        .get_or_create_stream(Config {
            name: EVENT_STREAM.to_owned(),
            subjects: vec![LIFECYCLE_EVENT_SUBJECT.to_owned()],
            retention: RetentionPolicy::Limits,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|error| TopologyError(error.to_string()))?;
    commands
        .get_or_create_consumer(
            COMMAND_CONSUMER,
            jetstream::consumer::pull::Config {
                durable_name: Some(COMMAND_CONSUMER.to_owned()),
                filter_subjects: vec![
                    String::from("heph.run.command.>"),
                    FORGE_START_RUN_SUBJECT.to_owned(),
                ],
                ack_wait: std::time::Duration::from_secs(30),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| TopologyError(error.to_string()))
}

/// `JetStream` topology configuration failure.
#[derive(Debug, thiserror::Error)]
#[error("JetStream topology configuration failed: {0}")]
pub struct TopologyError(String);

/// Decodes durable `JetStream` commands and acknowledges them after their
/// database effects are committed.
#[derive(Clone)]
pub struct NatsCommandHandler {
    orchestrator: Arc<RunOrchestrator>,
}

impl NatsCommandHandler {
    /// Creates a handler for a durable pull consumer.
    #[must_use]
    pub const fn new(orchestrator: Arc<RunOrchestrator>) -> Self {
        Self { orchestrator }
    }

    /// Serves durable deliveries with bounded concurrency.
    ///
    /// Start commands run concurrently so a cancellation delivery is not
    /// blocked behind a long-running VM. Failed commands remain unacknowledged
    /// and are eligible for `JetStream` redelivery.
    ///
    /// # Errors
    ///
    /// Returns an error when the durable delivery stream cannot be opened.
    pub async fn serve(
        &self,
        consumer: &jetstream::consumer::PullConsumer,
    ) -> Result<(), CommandConsumerError> {
        let messages = consumer
            .messages()
            .await
            .map_err(|error| CommandConsumerError(error.to_string()))?;
        messages
            .for_each_concurrent(Some(MAX_CONCURRENT_COMMANDS), |delivery| {
                let handler = self.clone();
                async move {
                    match delivery {
                        Ok(message) => {
                            if let Err(error) = handler.handle(&message).await {
                                tracing::warn!(%error, "run command was not acknowledged");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "failed to receive a run command");
                        }
                    }
                }
            })
            .await;
        Ok(())
    }

    /// Dispatches one delivery according to its versioned command subject.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown subject or any command-processing
    /// failure.
    pub async fn handle(&self, message: &jetstream::Message) -> Result<(), CommandHandlingError> {
        match message.message.subject.as_str() {
            START_RUN_SUBJECT | FORGE_START_RUN_SUBJECT => self.handle_start(message).await,
            CANCEL_RUN_SUBJECT => self.handle_cancel(message).await,
            subject => Err(CommandHandlingError::UnknownSubject(subject.to_owned())),
        }
    }

    /// Handles one `StartRun` delivery and uses a confirmed acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns without acknowledging when decoding or orchestration fails, so
    /// `JetStream` may redeliver the command.
    pub async fn handle_start(
        &self,
        message: &jetstream::Message,
    ) -> Result<(), CommandHandlingError> {
        let command: StartRun = serde_json::from_slice(&message.payload)?;
        let operation = self.orchestrator.start_run(&command);
        tokio::pin!(operation);
        let mut progress = tokio::time::interval(ACK_PROGRESS_INTERVAL);
        progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                result = &mut operation => {
                    result?;
                    break;
                }
                _ = progress.tick() => {
                    if let Err(error) = message.ack_with(jetstream::AckKind::Progress).await {
                        // Continue durable orchestration even if the NATS
                        // connection briefly cannot extend the ack deadline.
                        tracing::warn!(%error, "failed to acknowledge StartRun progress");
                    }
                }
            }
        }
        message
            .double_ack()
            .await
            .map_err(|error| CommandHandlingError::Acknowledgement(error.to_string()))
    }

    /// Handles one `CancelRun` delivery and uses a confirmed acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns without acknowledging when decoding or orchestration fails, so
    /// `JetStream` may redeliver the command.
    pub async fn handle_cancel(
        &self,
        message: &jetstream::Message,
    ) -> Result<(), CommandHandlingError> {
        let command: CancelRun = serde_json::from_slice(&message.payload)?;
        self.orchestrator.cancel_run(&command).await?;
        message
            .double_ack()
            .await
            .map_err(|error| CommandHandlingError::Acknowledgement(error.to_string()))
    }
}

/// Failure to open the durable command-delivery stream.
#[derive(Debug, thiserror::Error)]
#[error("JetStream command consumer failed: {0}")]
pub struct CommandConsumerError(String);

/// Durable command processing failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CommandHandlingError {
    /// The command payload was invalid.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// The durable orchestration operation failed.
    #[error(transparent)]
    Orchestration(#[from] OrchestratorError),
    /// `JetStream` did not confirm the consumer acknowledgement.
    #[error("JetStream acknowledgement failed: {0}")]
    Acknowledgement(String),
    /// The durable consumer received an unsupported subject.
    #[error("unsupported run-command subject {0}")]
    UnknownSubject(String),
}

/// Publishes transactional outbox records to NATS `JetStream`.
#[derive(Clone)]
pub struct NatsOutboxPublisher {
    context: jetstream::Context,
}

impl NatsOutboxPublisher {
    /// Creates a publisher from a `JetStream` context.
    #[must_use]
    pub const fn new(context: jetstream::Context) -> Self {
        Self { context }
    }

    /// Publishes up to `limit` pending records.
    ///
    /// The outbox identifier is sent as `Nats-Msg-Id`, making retries
    /// idempotent within `JetStream`'s configured duplicate window. Consumers
    /// still use the durable command inbox for unbounded idempotency.
    ///
    /// # Errors
    ///
    /// Returns an error when repository access, serialization, publication, or
    /// the `JetStream` acknowledgement fails.
    pub async fn publish_pending(
        &self,
        repository: &Arc<dyn RunRepository>,
        limit: i64,
    ) -> Result<usize, OutboxPublishError> {
        let records = repository.unpublished_outbox(limit).await?;
        let count = records.len();
        for record in records {
            let payload = serde_json::to_vec(&record.payload)?;
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", record.id.to_string());
            let publication = self
                .context
                .publish_with_headers(record.subject, headers, payload.into())
                .await;
            let result = match publication {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_acknowledgement) => {
                    repository.mark_outbox_published(record.id).await?;
                }
                Err(error) => {
                    let message = error.clone();
                    repository.mark_outbox_failed(record.id, &message).await?;
                    return Err(OutboxPublishError::JetStream(message));
                }
            }
        }
        Ok(count)
    }
}

/// Transactional outbox publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OutboxPublishError {
    /// Durable repository access failed.
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    /// Event serialization failed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` did not acknowledge publication.
    #[error("JetStream publication failed: {0}")]
    JetStream(String),
}

//! NATS transport for durable review and run commands.

use super::{
    CONTROL_EXECUTE_SUBJECT, ControlCommand, ControlHandlingError, ControlOutcome,
    ReviewControlService,
};
use async_nats::{HeaderMap, jetstream};
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

const COMMAND_SUBJECTS: [&str; 3] = [
    CONTROL_EXECUTE_SUBJECT,
    "heph.run.command.cancel.v1",
    "hephaestus.run.start",
];

/// Publishes browser control intents and their derived retry commands.
#[derive(Clone)]
pub struct ReviewOutboxPublisher {
    context: jetstream::Context,
    store: Arc<dyn ReviewOutboxStore>,
}

impl ReviewOutboxPublisher {
    /// Creates a publisher for review-originated commands.
    #[must_use]
    pub fn new(context: jetstream::Context, store: Arc<dyn ReviewOutboxStore>) -> Self {
        Self { context, store }
    }

    /// Publishes committed command records with `JetStream` deduplication.
    ///
    /// # Errors
    ///
    /// Returns after persisting the first database or publication failure.
    pub async fn publish_pending(&self, limit: i64) -> Result<usize, ReviewOutboxPublishError> {
        let rows = self.store.claim_pending(&COMMAND_SUBJECTS, limit).await?;
        let count = rows.len();
        for row in rows {
            publish_row(&self.context, self.store.as_ref(), row).await?;
        }
        Ok(count)
    }
}

/// Provider-neutral durable command publication record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewOutboxRecord {
    /// Durable message identifier used for broker deduplication.
    pub id: Uuid,
    /// Broker subject.
    pub subject: String,
    /// Serialized command payload.
    pub payload: serde_json::Value,
}

/// Persistence boundary for review-owned command outbox records.
#[async_trait]
pub trait ReviewOutboxStore: Send + Sync {
    /// Claims up to `limit` unpublished messages on the selected subjects.
    async fn claim_pending(
        &self,
        subjects: &[&str],
        limit: i64,
    ) -> Result<Vec<ReviewOutboxRecord>, ReviewOutboxStoreError>;

    /// Marks a broker-confirmed message as published.
    async fn mark_published(&self, id: Uuid) -> Result<(), ReviewOutboxStoreError>;

    /// Records a failed publication attempt.
    async fn mark_failed(&self, id: Uuid, error: &str) -> Result<(), ReviewOutboxStoreError>;
}

/// Provider-neutral outbox persistence failure.
#[derive(Debug, thiserror::Error)]
#[error("review outbox store failed: {0}")]
pub struct ReviewOutboxStoreError(pub String);

async fn publish_row(
    context: &jetstream::Context,
    store: &dyn ReviewOutboxStore,
    row: ReviewOutboxRecord,
) -> Result<(), ReviewOutboxPublishError> {
    let payload = serde_json::to_vec(&row.payload)?;
    let mut headers = HeaderMap::new();
    headers.insert("Nats-Msg-Id", row.id.to_string());
    let publication = context
        .publish_with_headers(row.subject, headers, payload.into())
        .await;
    let result = match publication {
        Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    match result {
        Ok(_) => {
            store.mark_published(row.id).await?;
            Ok(())
        }
        Err(error) => {
            store.mark_failed(row.id, &error).await?;
            Err(ReviewOutboxPublishError::JetStream(error))
        }
    }
}

/// Review command outbox database, serialization, or publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReviewOutboxPublishError {
    /// Durable outbox persistence failed.
    #[error(transparent)]
    Store(#[from] ReviewOutboxStoreError),
    /// Command serialization failed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` rejected publication.
    #[error("JetStream publication failed: {0}")]
    JetStream(String),
}

/// `JetStream` adapter which acknowledges only after durable command effects.
#[derive(Clone)]
pub struct NatsControlHandler {
    service: ReviewControlService,
}

impl NatsControlHandler {
    /// Creates a handler.
    #[must_use]
    pub const fn new(service: ReviewControlService) -> Self {
        Self { service }
    }

    /// Decodes, processes, and confirms one control delivery.
    ///
    /// # Errors
    ///
    /// Returns without acknowledging on a processing or acknowledgement
    /// failure, allowing `JetStream` redelivery.
    pub async fn handle(
        &self,
        message: &jetstream::Message,
    ) -> Result<ControlOutcome, ControlHandlingError> {
        if message.message.subject.as_str() != CONTROL_EXECUTE_SUBJECT {
            return Err(ControlHandlingError::UnknownSubject(
                message.message.subject.to_string(),
            ));
        }
        let command: ControlCommand = serde_json::from_slice(&message.payload)?;
        let result = self.service.execute(&command).await?;
        message
            .double_ack()
            .await
            .map_err(|error| ControlHandlingError::Acknowledgement(error.to_string()))?;
        Ok(result)
    }
}

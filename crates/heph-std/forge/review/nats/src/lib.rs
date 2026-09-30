//! NATS `JetStream` adapters for durable review commands.

use async_nats::{HeaderMap, jetstream};
use review_domain::{CONTROL_EXECUTE_SUBJECT, ControlCommand};
use review_service::{
    ControlOutcome, ControlServiceError, ReviewControlService, ReviewOutboxRecord,
    ReviewOutboxStore, ReviewOutboxStoreError,
};
use std::sync::Arc;

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

/// Control delivery failure at the NATS transport boundary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ControlHandlingError {
    /// Delivery used an unsupported subject.
    #[error("unsupported control subject {0}")]
    UnknownSubject(String),
    /// Delivery payload was not a valid command.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// Durable processing failed.
    #[error(transparent)]
    Service(#[from] ControlServiceError),
    /// `JetStream` did not confirm acknowledgement.
    #[error("control acknowledgement failed: {0}")]
    Acknowledgement(String),
}

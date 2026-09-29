use super::{
    MAILBOX_COMMAND_SUBJECTS,
    contract::{MailboxDispatchStore, MailboxDispatchStoreError},
};
use async_nats::{HeaderMap, jetstream};
use std::sync::Arc;

/// Publishes committed mailbox commands with `JetStream` deduplication.
#[derive(Clone)]
pub struct MailboxOutboxPublisher {
    context: jetstream::Context,
    store: Arc<dyn MailboxDispatchStore>,
}

impl MailboxOutboxPublisher {
    /// Creates a publisher using the authoritative mailbox store.
    #[must_use]
    pub fn new(context: jetstream::Context, store: Arc<dyn MailboxDispatchStore>) -> Self {
        Self { context, store }
    }

    /// Publishes up to `limit` committed commands.
    ///
    /// # Errors
    ///
    /// Returns after retaining the first broker or persistence failure.
    pub async fn publish_pending(&self, limit: i64) -> Result<usize, MailboxOutboxPublishError> {
        let rows = self
            .store
            .claim_outbox(&MAILBOX_COMMAND_SUBJECTS, limit)
            .await?;
        let count = rows.len();
        for row in rows {
            tracing::debug!(
                mailbox_event_id = %row.command.event_id,
                mailbox_operation_id = %row.command.operation_id,
                mailbox_outbox_id = %row.id,
                subject = %row.subject,
                "publishing durable mailbox command"
            );
            let payload = serde_json::to_vec(&row.command)?;
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", row.id.to_string());
            let result = match self
                .context
                .publish_with_headers(row.subject, headers, payload.into())
                .await
            {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_) => {
                    self.store
                        .mark_outbox_published(row.id, row.claim_token)
                        .await?;
                    tracing::debug!(
                        mailbox_event_id = %row.command.event_id,
                        mailbox_operation_id = %row.command.operation_id,
                        mailbox_outbox_id = %row.id,
                        "published durable mailbox command"
                    );
                }
                Err(error) => {
                    self.store
                        .mark_outbox_failed(row.id, row.claim_token, &error)
                        .await?;
                    return Err(MailboxOutboxPublishError::JetStream(error));
                }
            }
        }
        Ok(count)
    }
}

/// Mailbox command outbox or `JetStream` publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MailboxOutboxPublishError {
    /// The authoritative store failed.
    #[error(transparent)]
    Store(#[from] MailboxDispatchStoreError),
    /// The identifier-only command could not be serialized.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` rejected the publication.
    #[error("JetStream publication failed: {0}")]
    JetStream(String),
}

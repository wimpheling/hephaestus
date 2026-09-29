//! Provider-neutral durable command outbox contracts.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

/// Provider-neutral durable command publication record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewOutboxRecord {
    /// Durable message identifier used for broker deduplication.
    pub id: Uuid,
    /// Broker subject.
    pub subject: String,
    /// Serialized command payload.
    pub payload: Value,
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

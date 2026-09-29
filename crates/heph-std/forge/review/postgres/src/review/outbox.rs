//! Durable review command outbox access.

use async_trait::async_trait;
use review_service::{ReviewOutboxRecord, ReviewOutboxStore, ReviewOutboxStoreError};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::PostgresReviewRepository;
use super::support::store_error;

#[async_trait]
impl ReviewOutboxStore for PostgresReviewRepository {
    async fn claim_pending(
        &self,
        subjects: &[&str],
        limit: i64,
    ) -> Result<Vec<ReviewOutboxRecord>, ReviewOutboxStoreError> {
        let subjects: Vec<String> = subjects
            .iter()
            .map(|subject| (*subject).to_owned())
            .collect();
        sqlx::query_as::<_, OutboxRow>(
            "SELECT id, subject, payload FROM outbox
             WHERE published_at IS NULL AND subject = ANY($1)
             ORDER BY occurred_at, id LIMIT $2",
        )
        .bind(subjects)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(store_error)
    }

    async fn mark_published(&self, id: Uuid) -> Result<(), ReviewOutboxStoreError> {
        sqlx::query(
            "UPDATE outbox SET published_at = now(), attempts = attempts + 1,
                    last_error = NULL WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(store_error)
    }

    async fn mark_failed(&self, id: Uuid, error: &str) -> Result<(), ReviewOutboxStoreError> {
        sqlx::query("UPDATE outbox SET attempts = attempts + 1, last_error = $2 WHERE id = $1")
            .bind(id)
            .bind(error)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(store_error)
    }
}

#[derive(Debug, FromRow)]
struct OutboxRow {
    id: Uuid,
    subject: String,
    payload: Value,
}

impl From<OutboxRow> for ReviewOutboxRecord {
    fn from(row: OutboxRow) -> Self {
        Self {
            id: row.id,
            subject: row.subject,
            payload: row.payload,
        }
    }
}

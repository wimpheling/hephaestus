use super::{PgForgeRepository, helpers::storage, rows::OutboxRow};
use forge_service::{ForgeRepositoryError, OutboxRecord};
use runtime_types::EventId;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

impl PgForgeRepository {
    /// Loads unpublished actionable command entries.
    ///
    /// # Errors
    ///
    /// Returns an error when `PostgreSQL` access fails.
    pub async fn unpublished_outbox(
        &self,
        limit: i64,
    ) -> Result<Vec<OutboxRecord>, ForgeRepositoryError> {
        let rows = sqlx::query_as::<_, OutboxRow>(
            "SELECT id, subject, payload FROM outbox
             WHERE published_at IS NULL
               AND subject IN (
                   'hephaestus.build.requested.v1',
                   'hephaestus.instance.run.requested.v1',
                   'hephaestus.run.start'
               )
             ORDER BY occurred_at, id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        Ok(rows
            .into_iter()
            .map(|row| OutboxRecord {
                id: EventId::from_uuid(row.id),
                subject: row.subject,
                payload: row.payload,
            })
            .collect())
    }

    /// Marks an outbox event as acknowledged.
    ///
    /// # Errors
    ///
    /// Returns an error when `PostgreSQL` access fails.
    pub async fn mark_outbox_published(
        &self,
        event_id: EventId,
    ) -> Result<(), ForgeRepositoryError> {
        sqlx::query("UPDATE outbox SET published_at = now() WHERE id = $1")
            .bind(event_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// Records a failed publication attempt.
    ///
    /// # Errors
    ///
    /// Returns an error when `PostgreSQL` access fails.
    pub async fn mark_outbox_failed(
        &self,
        event_id: EventId,
        error: &str,
    ) -> Result<(), ForgeRepositoryError> {
        sqlx::query("UPDATE outbox SET attempts = attempts + 1, last_error = $2 WHERE id = $1")
            .bind(event_id.as_uuid())
            .bind(error)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }
}

pub async fn append_outbox(
    transaction: &mut Transaction<'_, Postgres>,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    mut payload: Value,
    occurred_at: OffsetDateTime,
) -> Result<(), ForgeRepositoryError> {
    let event_id = EventId::new();
    if let Some(object) = payload.as_object_mut() {
        object
            .entry(String::from("schema_version"))
            .or_insert_with(|| json!(1));
        object
            .entry(String::from("message_id"))
            .or_insert_with(|| json!(event_id));
        object
            .entry(String::from("idempotency_key"))
            .or_insert_with(|| json!(event_id));
        object
            .entry(String::from("request_id"))
            .or_insert(Value::Null);
        object
            .entry(String::from("trace_id"))
            .or_insert(Value::Null);
    }
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES ($1, 'forge', $2, $3, $4, $5, $6)",
    )
    .bind(event_id.as_uuid())
    .bind(aggregate_id)
    .bind(subject)
    .bind(event_type)
    .bind(payload)
    .bind(occurred_at)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}

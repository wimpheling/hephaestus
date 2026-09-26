//! `PostgreSQL` implementations of durable event application ports.

use async_trait::async_trait;
use event_application::{CommittedMutation, MutationReceiptError, MutationReceiptReader};
use event_application::{ProductEventOutbox, ProductEventRecord};
use identity_domain::{RequestId, UserId};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// PostgreSQL-backed committed mutation receipt reader.
#[derive(Clone)]
pub struct PostgresMutationReceiptReader {
    pool: PgPool,
}

impl PostgresMutationReceiptReader {
    /// Creates a receipt reader backed by the supplied event database pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MutationReceiptReader for PostgresMutationReceiptReader {
    async fn load(
        &self,
        occurrence_id: RequestId,
        actor_id: UserId,
        aggregate_type: &str,
        primary_scope_kind: &str,
    ) -> Result<CommittedMutation, MutationReceiptError> {
        sqlx::query_as::<_, MutationReceiptRow>(
            "SELECT id AS event_id, scope_kind, scope_id, cursor, aggregate_version
             FROM application_events
             WHERE occurrence_id = $1 AND aggregate_type = $2
               AND scope_kind = $3 AND actor_id = $4
             ORDER BY cursor DESC
             LIMIT 1",
        )
        .bind(occurrence_id.as_uuid())
        .bind(aggregate_type)
        .bind(primary_scope_kind)
        .bind(actor_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(MutationReceiptError::provider)?
        .map(CommittedMutation::from)
        .ok_or(MutationReceiptError::Missing)
    }
}

#[derive(FromRow)]
struct MutationReceiptRow {
    event_id: Uuid,
    scope_kind: String,
    scope_id: Uuid,
    cursor: i64,
    aggregate_version: i64,
}

impl From<MutationReceiptRow> for CommittedMutation {
    fn from(row: MutationReceiptRow) -> Self {
        Self {
            event_id: row.event_id,
            scope_kind: row.scope_kind,
            scope_id: row.scope_id,
            cursor: row.cursor,
            aggregate_version: row.aggregate_version,
        }
    }
}

mod outbox;
pub use outbox::{
    ReleaseOutboxPublishError, ReleaseOutboxPublisher, ensure_release_jetstream_topology,
};

/// `PostgreSQL` product-event outbox reader/updater.
#[derive(Clone)]
pub struct PostgresProductEventOutbox {
    pool: PgPool,
}

impl PostgresProductEventOutbox {
    /// Creates an outbox adapter.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProductEventOutbox for PostgresProductEventOutbox {
    async fn pending(&self, limit: i64) -> Result<Vec<ProductEventRecord>, MutationReceiptError> {
        let rows = sqlx::query_as::<_, ProductEventRow>("SELECT event.id,event.scope_kind,event.scope_id,event.cursor,event.aggregate_type,event.aggregate_id,event.aggregate_version,event.event_type,event.schema_version,event.change_kind,event.safe_state,event.related_id_one,event.related_id_two,event.actor_id,event.request_id,event.occurred_at FROM product_event_outbox outbox JOIN application_events event ON event.id=outbox.event_id WHERE outbox.published_at IS NULL AND outbox.dead_lettered_at IS NULL ORDER BY event.scope_kind,event.scope_id,event.cursor LIMIT $1").bind(limit).fetch_all(&self.pool).await.map_err(MutationReceiptError::provider)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
    async fn mark_published(&self, event_id: Uuid) -> Result<(), MutationReceiptError> {
        sqlx::query("UPDATE product_event_outbox SET published_at=COALESCE(published_at,now()),last_error=NULL WHERE event_id=$1").bind(event_id).execute(&self.pool).await.map_err(MutationReceiptError::provider).map(|_| ())
    }
    async fn mark_failed(&self, event_id: Uuid, message: &str) -> Result<(), MutationReceiptError> {
        sqlx::query("UPDATE product_event_outbox SET attempts=attempts+1,last_error=left($2,2048) WHERE event_id=$1 AND published_at IS NULL").bind(event_id).bind(message).execute(&self.pool).await.map_err(MutationReceiptError::provider).map(|_| ())
    }
    async fn dead_letter(&self, event_id: Uuid, reason: &str) -> Result<(), MutationReceiptError> {
        sqlx::query("UPDATE product_event_outbox SET dead_lettered_at=COALESCE(dead_lettered_at,now()),terminal_reason=left($2,2048),last_error=left($2,2048) WHERE event_id=$1 AND published_at IS NULL")
            .bind(event_id)
            .bind(reason)
            .execute(&self.pool)
            .await
            .map_err(MutationReceiptError::provider)
            .map(|_| ())
    }
}

#[derive(sqlx::FromRow)]
struct ProductEventRow {
    id: Uuid,
    scope_kind: String,
    scope_id: Uuid,
    cursor: i64,
    aggregate_type: String,
    aggregate_id: Uuid,
    aggregate_version: i64,
    event_type: String,
    schema_version: i32,
    change_kind: String,
    safe_state: Option<String>,
    related_id_one: Option<Uuid>,
    related_id_two: Option<Uuid>,
    actor_id: Option<Uuid>,
    request_id: Option<Uuid>,
    occurred_at: time::OffsetDateTime,
}
impl From<ProductEventRow> for ProductEventRecord {
    fn from(row: ProductEventRow) -> Self {
        Self {
            id: row.id,
            scope_kind: row.scope_kind,
            scope_id: row.scope_id,
            cursor: row.cursor,
            aggregate_type: row.aggregate_type,
            aggregate_id: row.aggregate_id,
            aggregate_version: row.aggregate_version,
            event_type: row.event_type,
            schema_version: row.schema_version,
            change_kind: row.change_kind,
            safe_state: row.safe_state,
            related_id_one: row.related_id_one,
            related_id_two: row.related_id_two,
            actor_id: row.actor_id,
            request_id: row.request_id,
            occurred_at: row.occurred_at,
        }
    }
}

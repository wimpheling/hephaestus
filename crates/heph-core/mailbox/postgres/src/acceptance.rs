use mailbox_domain::{MailboxEvent, MailboxEventId};
use uuid::Uuid;

use crate::{
    AcceptedMailboxEvent, MailboxPersistenceError, PostgresMailboxRepository, errors::storage,
    helpers::validate_payload,
};
impl PostgresMailboxRepository {
    /// Accepts one event and its encoded body exactly once per producer key.
    ///
    /// A duplicate producer-scoped key returns the original accepted event.
    /// The losing concurrent transaction rolls back its candidate body before
    /// looking up that original, so it cannot leave an orphaned payload.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxPersistenceError`] for unavailable mailboxes, malformed
    /// payload evidence, or provider failures.
    pub async fn accept(
        &self,
        project_id: Uuid,
        event: &MailboxEvent,
        encoded_body: &[u8],
        decoded_length: u32,
    ) -> Result<AcceptedMailboxEvent, MailboxPersistenceError> {
        validate_payload(&event.envelope.content, encoded_body, decoded_length)?;
        let body = &event.envelope.content.body;
        let headers = serde_json::to_value(&event.envelope.headers).map_err(storage)?;
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        sqlx::query(
            "INSERT INTO mailbox_payloads
                 (id, mailbox_id, project_id, encoded_body, encoded_length,
                  decoded_length, integrity_hash, content_type, content_encoding)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(body.id.as_uuid())
        .bind(event.mailbox_id.as_uuid())
        .bind(project_id)
        .bind(encoded_body)
        .bind(i32::try_from(encoded_body.len()).map_err(storage)?)
        .bind(i32::try_from(decoded_length).map_err(storage)?)
        .bind(body.integrity_hash.as_slice())
        .bind(&event.envelope.content.content_type)
        .bind(&event.envelope.content.content_encoding)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        let inserted = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO mailbox_events
                 (id, mailbox_id, project_id, instance_id, body_id,
                  producer_kind, producer_id, deduplication_scope,
                  deduplication_key, method, route, selected_headers,
                  content_type, received_at, trace_context)
             VALUES ($1, $2, $3, $4, $5, 'external', $6, $6, $7, $8, $9,
                     $10, $11, $12, $13)
             ON CONFLICT (mailbox_id, deduplication_scope, deduplication_key)
             DO NOTHING
             RETURNING id",
        )
        .bind(event.id.as_uuid())
        .bind(event.mailbox_id.as_uuid())
        .bind(project_id)
        .bind(event.instance_id.as_uuid())
        .bind(body.id.as_uuid())
        .bind(event.producer_id.as_str())
        .bind(event.deduplication_key.as_str())
        .bind(event.envelope.method.as_str())
        .bind(event.envelope.route.as_str())
        .bind(headers)
        .bind(&event.envelope.content.content_type)
        .bind(event.envelope.received_at)
        .bind(
            event
                .envelope
                .trace_context
                .as_ref()
                .map(mailbox_domain::TraceContext::as_str),
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        if let Some(event_id) = inserted {
            transaction.commit().await.map_err(storage)?;
            return Ok(AcceptedMailboxEvent {
                event_id: MailboxEventId::from_uuid(event_id),
                duplicate: false,
            });
        }
        transaction.rollback().await.map_err(storage)?;
        let event_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM mailbox_events
             WHERE mailbox_id = $1 AND deduplication_scope = $2
               AND deduplication_key = $3",
        )
        .bind(event.mailbox_id.as_uuid())
        .bind(event.producer_id.as_str())
        .bind(event.deduplication_key.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(MailboxPersistenceError::Unavailable)?;
        Ok(AcceptedMailboxEvent {
            event_id: MailboxEventId::from_uuid(event_id),
            duplicate: true,
        })
    }
}

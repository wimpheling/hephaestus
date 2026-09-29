use mailbox_dispatch::{
    MAILBOX_DISPATCH_SUBJECT, MAILBOX_RETRY_SUBJECT, MailboxDispatchStoreError,
};
use mailbox_domain::{ContentMetadata, MailboxEventId, MailboxId, MailboxOperationIdentity};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    MailboxPersistenceError,
    errors::{dispatch_error, storage},
};

pub fn allocation_command_key(identity: &identity_domain::AuthenticatedIdentity) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus-mailbox-allocation-v1\0");
    digest.update(identity.idempotency_id.as_uuid().as_bytes());
    digest.finalize().into()
}

pub fn validate_payload(
    content: &ContentMetadata,
    encoded_body: &[u8],
    decoded_length: u32,
) -> Result<(), MailboxPersistenceError> {
    let body = &content.body;
    if encoded_body.len() != usize::try_from(body.byte_length).map_err(storage)?
        || decoded_length != body.byte_length
        || Sha256::digest(encoded_body).as_slice() != body.integrity_hash
    {
        return Err(MailboxPersistenceError::PayloadIntegrity);
    }
    if content
        .content_encoding
        .as_deref()
        .is_some_and(|value| value != "identity")
    {
        return Err(MailboxPersistenceError::UnsupportedContentEncoding);
    }
    Ok(())
}
/// The mailbox dispatcher uses a narrow worker-only view of one selected
/// revision.  Selecting it under the delivery row lock prevents a queued event
/// from silently following a later instance revision.
#[derive(Debug, FromRow)]
pub struct DispatchTargetRow {
    pub mailbox_id: Uuid,
    pub instance_id: Uuid,
    pub instance_revision_id: Uuid,
    pub release_id: Uuid,
    pub release_agent_id: Uuid,
    pub attachment_id: Uuid,
    pub target_ref: String,
    pub target_commit: String,
    pub requires_state: bool,
    pub logical_attempt_count: i32,
}

#[derive(Debug, FromRow)]
pub struct OutboxRow {
    pub id: Uuid,
    pub claim_token: Uuid,
    pub subject: String,
    pub operation_id: Uuid,
    pub event_id: Uuid,
}
pub async fn worker_transaction(
    pool: &PgPool,
) -> Result<Transaction<'_, Postgres>, MailboxDispatchStoreError> {
    let mut transaction = pool.begin().await.map_err(dispatch_error)?;
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
    Ok(transaction)
}

/// Atomically proves that this worker still owns an unexpired claim before it
/// changes broker-publication state. A delayed publisher cannot settle a claim
/// that a recovery worker has already replaced.
pub async fn take_outbox_claim(
    transaction: &mut Transaction<'_, Postgres>,
    id: Uuid,
    claim_token: Uuid,
) -> Result<(), MailboxDispatchStoreError> {
    let taken = sqlx::query_scalar::<_, Uuid>(
        "DELETE FROM mailbox_outbox_claims
         WHERE outbox_id = $1 AND claim_token = $2 AND expires_at > now()
         RETURNING outbox_id",
    )
    .bind(id)
    .bind(claim_token)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(dispatch_error)?;
    if taken.is_some() {
        Ok(())
    } else {
        Err(MailboxDispatchStoreError(
            "mailbox outbox claim is no longer current".to_owned(),
        ))
    }
}

pub async fn enqueue_command(
    transaction: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    event_id: Uuid,
    subject: &str,
    event_type: &str,
) -> Result<bool, MailboxDispatchStoreError> {
    enqueue_command_with_transport_id(
        transaction,
        operation_id,
        operation_id,
        event_id,
        subject,
        event_type,
    )
    .await
}

pub async fn enqueue_command_with_transport_id(
    transaction: &mut Transaction<'_, Postgres>,
    transport_id: Uuid,
    operation_id: Uuid,
    event_id: Uuid,
    subject: &str,
    event_type: &str,
) -> Result<bool, MailboxDispatchStoreError> {
    sqlx::query(
        "INSERT INTO outbox (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES ($1, 'mailbox_event', $2, $3, $4,
                 jsonb_build_object('operation_id', $5, 'mailbox_event_id', $2), now())
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(transport_id)
    .bind(event_id)
    .bind(subject)
    .bind(event_type)
    .bind(operation_id)
    .execute(&mut **transaction)
    .await
    .map(|result| result.rows_affected() == 1)
    .map_err(dispatch_error)
}

/// Enqueues the next logical dispatch after a wake made a delivery eligible.
///
/// The logical operation remains deterministic for the attempt number.  If a
/// prior transport message with that operation was already consumed, the
/// replacement gets a distinct outbox/NATS message ID while carrying the same
/// operation payload, preserving the claim identity check.
pub async fn enqueue_next_dispatch(
    transaction: &mut Transaction<'_, Postgres>,
    mailbox_id: Uuid,
    event_id: MailboxEventId,
    wake_operation_id: Uuid,
) -> Result<(), MailboxDispatchStoreError> {
    let current_attempt = sqlx::query_scalar::<_, i32>(
        "SELECT logical_attempt_count FROM mailbox_deliveries WHERE event_id = $1",
    )
    .bind(event_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(dispatch_error)?;
    let next_attempt = current_attempt
        .checked_add(1)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| MailboxDispatchStoreError("mailbox attempt limit exceeded".to_owned()))?;
    let operation_id = MailboxOperationIdentity::dispatch(
        MailboxId::from_uuid(mailbox_id),
        event_id,
        next_attempt,
    )
    .id()
    .as_uuid();
    enqueue_command_with_transport_id(
        transaction,
        deterministic_transport_id(wake_operation_id, operation_id),
        operation_id,
        event_id.as_uuid(),
        MAILBOX_DISPATCH_SUBJECT,
        "mailbox.dispatch.v1",
    )
    .await?;
    Ok(())
}

const fn mailbox_transport_namespace() -> Uuid {
    Uuid::from_u128(0x8a3c_7b2e_5d91_4f60_9c17_2a6e_b4d8_f031)
}

/// Derives a stable transport identity for one wake and logical dispatch.
/// The wake operation separates transport retries from later activation
/// wakes, while the logical operation keeps the claimed attempt deterministic.
pub fn deterministic_transport_id(wake_operation_id: Uuid, dispatch_operation_id: Uuid) -> Uuid {
    let mut name = Vec::with_capacity(64);
    name.extend_from_slice(b"mailbox-dispatch-transport.v1\0");
    name.extend_from_slice(wake_operation_id.as_bytes());
    name.extend_from_slice(dispatch_operation_id.as_bytes());
    Uuid::new_v5(&mailbox_transport_namespace(), &name)
}

/// Schedules the deterministic wake-up for the next attempt in the current
/// worker transaction. The delivery must already be in `retryable` state.
pub async fn schedule_retry(
    transaction: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    mailbox_id: Uuid,
    attempt_number: i32,
) -> Result<(), MailboxDispatchStoreError> {
    let attempt_number = u32::try_from(attempt_number).map_err(dispatch_error)?;
    let retry = MailboxOperationIdentity::retry(
        MailboxId::from_uuid(mailbox_id),
        MailboxEventId::from_uuid(event_id),
        attempt_number,
    )
    .id();
    enqueue_command(
        transaction,
        retry.as_uuid(),
        event_id,
        MAILBOX_RETRY_SUBJECT,
        "mailbox.retry.v1",
    )
    .await
    .map(|_| ())
}

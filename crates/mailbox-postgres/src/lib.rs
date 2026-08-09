//! PostgreSQL-backed atomic acceptance of bounded durable mailbox events.
//!
//! The adapter persists opaque body bytes and their envelope in one database
//! transaction. The database trigger then creates the initial delivery row and
//! identifier-only transactional-outbox wake command in that same commit.

use async_trait::async_trait;
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use mailbox_dispatch::{
    MAILBOX_CANCEL_SUBJECT, MAILBOX_DISPATCH_SUBJECT, MAILBOX_RECOVERY_SUBJECT,
    MAILBOX_RETRY_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchCommand, MailboxDispatchStore,
    MailboxDispatchStoreError, MailboxOutboxRecord,
};
use mailbox_domain::{
    ContentMetadata, MailboxEvent, MailboxEventId, MailboxId, MailboxOperationIdentity,
};
use run_domain::{Run, RunKind, RunOutcome, RunState, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

/// `PostgreSQL` authority for mailbox creation and event acceptance.
#[derive(Clone)]
pub struct PostgresMailboxRepository {
    pool: PgPool,
}

/// Explicit operator action retained independently of opaque mailbox bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailboxOperatorAction {
    /// Stop new dispatch claims for a mailbox without dropping accepted work.
    Pause,
    /// Re-open a paused mailbox and wake its deferred deliveries.
    Resume,
    /// Make one retryable or denied delivery eligible immediately.
    Retry,
    /// Prevent a non-terminal delivery from starting again.
    Cancel,
    /// Retain a non-terminal delivery without further automatic execution.
    DeadLetter,
}

impl MailboxOperatorAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Retry => "retry",
            Self::Cancel => "cancel",
            Self::DeadLetter => "dead_letter",
        }
    }
}

/// Redacted result of an authorized mailbox control action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxOperatorResult {
    /// Whether the requested authoritative state transition occurred.
    pub changed: bool,
}

impl PostgresMailboxRepository {
    /// Creates a mailbox adapter over the control-plane pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Ensures the supplied mailbox is owned by exactly one project instance.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxPersistenceError`] when the durable ownership boundary
    /// rejects the requested mailbox.
    pub async fn ensure_mailbox(
        &self,
        project_id: Uuid,
        mailbox_id: MailboxId,
        instance_id: AgentInstanceId,
    ) -> Result<(), MailboxPersistenceError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        // A JetStream dispatch command remains stable across redelivery, but a
        // retry is a distinct logical VM execution.  The run table enforces a
        // globally unique command ID, so bind that command to the deterministic
        // attempt identity rather than the stable dispatch-command identity.
        sqlx::query(
            "INSERT INTO mailboxes (id, project_id, instance_id, state)
             VALUES ($1, $2, $3, 'active')
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(mailbox_id.as_uuid())
        .bind(project_id)
        .bind(instance_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        let owned: bool = sqlx::query_scalar(
            "SELECT project_id = $2 AND instance_id = $3
             FROM mailboxes WHERE id = $1",
        )
        .bind(mailbox_id.as_uuid())
        .bind(project_id)
        .bind(instance_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .unwrap_or(false);
        if owned {
            transaction.commit().await.map_err(storage)?;
            Ok(())
        } else {
            transaction.rollback().await.map_err(storage)?;
            Err(MailboxPersistenceError::Unavailable)
        }
    }

    /// Applies an authorized operator action without exposing the event body.
    ///
    /// The permission decision, state transition, command enqueue, and audit
    /// record share one transaction.  Operators need `can_recover` on the
    /// owning agent instance; callers without that permission receive the same
    /// non-disclosing unavailable result as an unknown event.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxPersistenceError`] when the request shape is invalid,
    /// the actor lacks recovery authority, or the durable transaction fails.
    // This is intentionally one transaction so authorization, worker-only
    // transition, outbox enqueue, and audit cannot be observed separately.
    #[allow(clippy::too_many_lines)]
    pub async fn operate(
        &self,
        identity: &AuthenticatedIdentity,
        action: MailboxOperatorAction,
        mailbox_id: MailboxId,
        event_id: Option<MailboxEventId>,
    ) -> Result<MailboxOperatorResult, MailboxPersistenceError> {
        if matches!(
            action,
            MailboxOperatorAction::Retry
                | MailboxOperatorAction::Cancel
                | MailboxOperatorAction::DeadLetter
        ) && event_id.is_none()
        {
            return Err(MailboxPersistenceError::Unavailable);
        }
        if matches!(
            action,
            MailboxOperatorAction::Pause | MailboxOperatorAction::Resume
        ) && event_id.is_some()
        {
            return Err(MailboxPersistenceError::Unavailable);
        }
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        sqlx::query("SET LOCAL ROLE hephaestus_app")
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        let allowed: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM mailboxes AS mailbox
                WHERE mailbox.id = $1
                  AND check_permission('user', hephaestus_actor_id(), 'can_recover',
                        'agent_instance', mailbox.instance_id::text) = 1
             )",
        )
        .bind(mailbox_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage)?;
        if !allowed {
            return Err(MailboxPersistenceError::Unavailable);
        }
        // The application role has proved the actor's exact recovery authority.
        // The following narrow mutation is executed as the worker because RLS
        // intentionally does not grant users raw state-machine writes.
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        let changed = match action {
            MailboxOperatorAction::Pause => {
                sqlx::query(
                    "UPDATE mailboxes SET state = 'paused', updated_at = now()
                 WHERE id = $1 AND state = 'active'",
                )
                .bind(mailbox_id.as_uuid())
                .execute(&mut *transaction)
                .await
                .map_err(storage)?
                .rows_affected()
                    == 1
            }
            MailboxOperatorAction::Resume => {
                let changed = sqlx::query(
                    "UPDATE mailboxes SET state = 'active', updated_at = now()
                     WHERE id = $1 AND state = 'paused'",
                )
                .bind(mailbox_id.as_uuid())
                .execute(&mut *transaction)
                .await
                .map_err(storage)?
                .rows_affected()
                    == 1;
                if changed {
                    let rows = sqlx::query_scalar::<_, Uuid>(
                        "SELECT event_id FROM mailbox_deliveries
                         WHERE mailbox_id = $1 AND disposition IN ('pending', 'eligible', 'retryable')",
                    ).bind(mailbox_id.as_uuid()).fetch_all(&mut *transaction).await.map_err(storage)?;
                    for id in rows {
                        enqueue_command(
                            &mut transaction,
                            Uuid::new_v4(),
                            id,
                            MAILBOX_WAKE_SUBJECT,
                            "mailbox.wake.v1",
                        )
                        .await
                        .map_err(|error| MailboxPersistenceError::Provider(error.to_string()))?;
                    }
                }
                changed
            }
            MailboxOperatorAction::Retry => {
                let id = event_id.ok_or(MailboxPersistenceError::Unavailable)?;
                let changed = sqlx::query(
                    "UPDATE mailbox_deliveries SET disposition = 'retryable', next_eligible_at = now(),
                         terminal_at = NULL, denial_code = NULL, updated_at = now()
                     WHERE event_id = $1 AND mailbox_id = $2
                       AND disposition IN ('retryable', 'denied')",
                ).bind(id.as_uuid()).bind(mailbox_id.as_uuid()).execute(&mut *transaction).await.map_err(storage)?.rows_affected() == 1;
                if changed {
                    enqueue_command(
                        &mut transaction,
                        Uuid::new_v4(),
                        id.as_uuid(),
                        MAILBOX_RETRY_SUBJECT,
                        "mailbox.retry.v1",
                    )
                    .await
                    .map_err(|error| MailboxPersistenceError::Provider(error.to_string()))?;
                }
                changed
            }
            MailboxOperatorAction::Cancel => {
                let id = event_id.ok_or(MailboxPersistenceError::Unavailable)?;
                let changed = sqlx::query(
                    "UPDATE mailbox_deliveries SET disposition = 'cancelled', terminal_at = now(), updated_at = now()
                     WHERE event_id = $1 AND mailbox_id = $2
                       AND disposition NOT IN ('delivered', 'dead_lettered', 'cancelled')",
                ).bind(id.as_uuid()).bind(mailbox_id.as_uuid()).execute(&mut *transaction).await.map_err(storage)?.rows_affected() == 1;
                if changed {
                    enqueue_command(
                        &mut transaction,
                        Uuid::new_v4(),
                        id.as_uuid(),
                        MAILBOX_CANCEL_SUBJECT,
                        "mailbox.cancel.v1",
                    )
                    .await
                    .map_err(|error| MailboxPersistenceError::Provider(error.to_string()))?;
                }
                changed
            }
            MailboxOperatorAction::DeadLetter => {
                let id = event_id.ok_or(MailboxPersistenceError::Unavailable)?;
                sqlx::query(
                    "UPDATE mailbox_deliveries SET disposition = 'dead_lettered', terminal_at = now(), updated_at = now()
                     WHERE event_id = $1 AND mailbox_id = $2
                       AND disposition NOT IN ('delivered', 'dead_lettered', 'cancelled')",
                ).bind(id.as_uuid()).bind(mailbox_id.as_uuid()).execute(&mut *transaction).await.map_err(storage)?.rows_affected() == 1
            }
        };
        sqlx::query(
            "INSERT INTO mailbox_operator_audit
                 (id, mailbox_id, event_id, actor_id, request_id, action, changed)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(mailbox_id.as_uuid())
        .bind(event_id.map(MailboxEventId::as_uuid))
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .bind(action.as_str())
        .bind(changed)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(MailboxOperatorResult { changed })
    }

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

/// The acceptance result, independent of a transport delivery outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedMailboxEvent {
    /// Immutable accepted event identity.
    pub event_id: MailboxEventId,
    /// Whether this call observed an already-accepted producer key.
    pub duplicate: bool,
}

/// Non-disclosing mailbox persistence failure.
#[derive(Debug, thiserror::Error)]
pub enum MailboxPersistenceError {
    /// The mailbox is unavailable or a duplicate cannot be read back safely.
    #[error("mailbox is unavailable")]
    Unavailable,
    /// The presented encoded body does not match its immutable body reference.
    #[error("mailbox payload integrity validation failed")]
    PayloadIntegrity,
    /// The bounded MVP-02 payload store deliberately accepts identity only.
    #[error("mailbox payload content encoding is unsupported")]
    UnsupportedContentEncoding,
    /// `PostgreSQL` or serialization failed without exposing request content.
    #[error("mailbox persistence failed: {0}")]
    Provider(String),
}

fn storage(error: impl std::fmt::Display) -> MailboxPersistenceError {
    MailboxPersistenceError::Provider(error.to_string())
}

fn validate_payload(
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
struct DispatchTargetRow {
    mailbox_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Uuid,
    requires_state: bool,
    logical_attempt_count: i32,
}

#[derive(Debug, FromRow)]
struct OutboxRow {
    id: Uuid,
    claim_token: Uuid,
    subject: String,
    operation_id: Uuid,
    event_id: Uuid,
}

#[async_trait]
impl MailboxDispatchStore for PostgresMailboxRepository {
    async fn claim_outbox(
        &self,
        subjects: &[&str],
        limit: i64,
    ) -> Result<Vec<MailboxOutboxRecord>, MailboxDispatchStoreError> {
        let subjects = subjects.iter().map(ToString::to_string).collect::<Vec<_>>();
        let mut transaction = worker_transaction(&self.pool).await?;
        sqlx::query("DELETE FROM mailbox_outbox_claims WHERE expires_at <= now()")
            .execute(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
        let rows = sqlx::query_as::<_, OutboxRow>(
            "WITH candidates AS (
                SELECT outbox.id FROM outbox
                LEFT JOIN mailbox_outbox_claims AS claim ON claim.outbox_id = outbox.id
                WHERE outbox.published_at IS NULL AND outbox.subject = ANY($1)
                  AND claim.outbox_id IS NULL
                ORDER BY outbox.occurred_at, outbox.id
                LIMIT $2 FOR UPDATE OF outbox SKIP LOCKED
             ), claimed AS (
                INSERT INTO mailbox_outbox_claims (outbox_id, claim_token, expires_at)
                SELECT id, gen_random_uuid(), now() + interval '5 minutes' FROM candidates
                ON CONFLICT (outbox_id) DO NOTHING RETURNING outbox_id, claim_token
             )
             SELECT outbox.id, claimed.claim_token, outbox.subject,
                    (outbox.payload->>'operation_id')::uuid AS operation_id,
                    (outbox.payload->>'mailbox_event_id')::uuid AS event_id
             FROM outbox JOIN claimed ON claimed.outbox_id = outbox.id
             ORDER BY outbox.occurred_at, outbox.id",
        )
        .bind(subjects)
        .bind(limit.clamp(1, 1_000))
        .fetch_all(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        transaction.commit().await.map_err(dispatch_error)?;
        Ok(rows
            .into_iter()
            .map(|row| MailboxOutboxRecord {
                id: row.id,
                claim_token: row.claim_token,
                subject: row.subject,
                command: MailboxDispatchCommand {
                    operation_id: mailbox_domain::MailboxOperationId::from_uuid(row.operation_id),
                    event_id: MailboxEventId::from_uuid(row.event_id),
                },
            })
            .collect())
    }

    async fn mark_outbox_published(
        &self,
        id: Uuid,
        claim_token: Uuid,
    ) -> Result<(), MailboxDispatchStoreError> {
        let mut transaction = worker_transaction(&self.pool).await?;
        take_outbox_claim(&mut transaction, id, claim_token).await?;
        sqlx::query(
            "UPDATE outbox SET published_at = COALESCE(published_at, now()),
                    attempts = attempts + 1, last_error = NULL WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        transaction.commit().await.map_err(dispatch_error)
    }

    async fn mark_outbox_failed(
        &self,
        id: Uuid,
        claim_token: Uuid,
        error: &str,
    ) -> Result<(), MailboxDispatchStoreError> {
        let mut transaction = worker_transaction(&self.pool).await?;
        take_outbox_claim(&mut transaction, id, claim_token).await?;
        sqlx::query("UPDATE outbox SET attempts = attempts + 1, last_error = $2 WHERE id = $1")
            .bind(id)
            .bind(error)
            .execute(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
        transaction.commit().await.map_err(dispatch_error)
    }

    async fn apply_command(
        &self,
        subject: &str,
        command: &MailboxDispatchCommand,
    ) -> Result<(), MailboxDispatchStoreError> {
        let mut transaction = worker_transaction(&self.pool).await?;
        // JetStream is only a delivery mechanism.  Treat a command as
        // authoritative only when it is the exact identifier-only record that
        // was committed through the transactional outbox.  This prevents a
        // producer with access to the internal subject from selecting another
        // event or operation by constructing a plausible payload.
        let committed = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                SELECT 1 FROM outbox
                WHERE id = $1 AND subject = $2
                  AND payload->>'operation_id' = $1::text
                  AND payload->>'mailbox_event_id' = $3::text
             )",
        )
        .bind(command.operation_id.as_uuid())
        .bind(subject)
        .bind(command.event_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        if !committed {
            return Err(MailboxDispatchStoreError(
                "mailbox command is not a committed outbox operation".to_owned(),
            ));
        }
        let mailbox_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT mailbox_id FROM mailbox_events WHERE id = $1 FOR KEY SHARE",
        )
        .bind(command.event_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(dispatch_error)?
        .ok_or_else(|| MailboxDispatchStoreError("unknown mailbox event".to_owned()))?;

        let changed = match subject {
            MAILBOX_WAKE_SUBJECT | MAILBOX_RETRY_SUBJECT | MAILBOX_RECOVERY_SUBJECT => {
                sqlx::query(
                    "UPDATE mailbox_deliveries
                     SET disposition = 'eligible', next_eligible_at = NULL, updated_at = now()
                     WHERE event_id = $1
                       AND (disposition IN ('pending', 'eligible')
                            OR (disposition = 'retryable' AND next_eligible_at <= now()))",
                )
                .bind(command.event_id.as_uuid())
                .execute(&mut *transaction)
                .await
                .map_err(dispatch_error)?
                .rows_affected()
                    == 1
            }
            MAILBOX_CANCEL_SUBJECT => {
                sqlx::query(
                    "UPDATE mailbox_deliveries
                 SET disposition = 'cancelled', terminal_at = now(), updated_at = now()
                 WHERE event_id = $1
                   AND disposition NOT IN ('delivered', 'denied', 'dead_lettered', 'cancelled')",
                )
                .bind(command.event_id.as_uuid())
                .execute(&mut *transaction)
                .await
                .map_err(dispatch_error)?
                .rows_affected()
                    == 1
            }
            // Dispatch has no separate pre-claim state transition: the
            // following `claim_dispatch` transaction atomically rechecks
            // eligibility and creates the one durable run.  Accepting this
            // committed identifier-only command here lets the JetStream
            // handler acknowledge only after that compare-and-swap succeeds.
            MAILBOX_DISPATCH_SUBJECT => false,
            _ => {
                return Err(MailboxDispatchStoreError(
                    "unsupported mailbox subject".to_owned(),
                ));
            }
        };
        if changed && subject != MAILBOX_CANCEL_SUBJECT {
            let current_attempt = sqlx::query_scalar::<_, i32>(
                "SELECT logical_attempt_count FROM mailbox_deliveries WHERE event_id = $1",
            )
            .bind(command.event_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
            let next_attempt = current_attempt
                .checked_add(1)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    MailboxDispatchStoreError("mailbox attempt limit exceeded".to_owned())
                })?;
            let operation_id = MailboxOperationIdentity::dispatch(
                MailboxId::from_uuid(mailbox_id),
                command.event_id,
                next_attempt,
            )
            .id();
            enqueue_command(
                &mut transaction,
                operation_id.as_uuid(),
                command.event_id.as_uuid(),
                MAILBOX_DISPATCH_SUBJECT,
                "mailbox.dispatch.v1",
            )
            .await?;
        }
        transaction.commit().await.map_err(dispatch_error)
    }

    // One transaction deliberately keeps the eligibility recheck, per-instance
    // serialization, attempt, and pre-created run adjacent for auditability.
    #[allow(clippy::too_many_lines)]
    async fn claim_dispatch(
        &self,
        command: &MailboxDispatchCommand,
    ) -> Result<Option<StartRun>, MailboxDispatchStoreError> {
        let mut transaction = worker_transaction(&self.pool).await?;
        let target = sqlx::query_as::<_, DispatchTargetRow>(
            "SELECT event.mailbox_id, event.instance_id, revision.id AS instance_revision_id,
                    release_agent.release_id, revision.release_agent_id,
                    attachment.id AS attachment_id, release_agent.requires_state,
                    delivery.logical_attempt_count
             FROM mailbox_deliveries AS delivery
             JOIN mailbox_events AS event ON event.id = delivery.event_id
             JOIN mailboxes AS mailbox ON mailbox.id = event.mailbox_id
             JOIN agent_instances AS instance ON instance.id = event.instance_id
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id AND revision.instance_id = instance.id
             JOIN release_agents AS release_agent ON release_agent.id = revision.release_agent_id
             JOIN releases AS release ON release.id = release_agent.release_id
             JOIN LATERAL (
                SELECT id FROM agent_attachments
                WHERE instance_id = instance.id AND enabled AND removed_at IS NULL
                ORDER BY created_at, id LIMIT 1
             ) AS attachment ON true
             WHERE delivery.event_id = $1 AND delivery.disposition = 'eligible'
               AND mailbox.state = 'active' AND instance.run_gate_open
               AND instance.state IN ('active', 'update_rejected')
               AND revision.runnable AND release.state = 'published'
             FOR UPDATE OF delivery",
        )
        .bind(command.event_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        let Some(target) = target else {
            transaction.commit().await.map_err(dispatch_error)?;
            return Ok(None);
        };
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(target.instance_id)
            .execute(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
        let attempt_number = target.logical_attempt_count.checked_add(1).ok_or_else(|| {
            MailboxDispatchStoreError("mailbox attempt limit exceeded".to_owned())
        })?;
        if attempt_number > 100 {
            transaction.commit().await.map_err(dispatch_error)?;
            return Ok(None);
        }
        let attempt_number_u32 = u32::try_from(attempt_number).map_err(dispatch_error)?;
        let expected_dispatch = MailboxOperationIdentity::dispatch(
            MailboxId::from_uuid(target.mailbox_id),
            command.event_id,
            attempt_number_u32,
        )
        .id();
        if command.operation_id != expected_dispatch {
            return Err(MailboxDispatchStoreError(
                "mailbox dispatch command does not match the next logical attempt".to_owned(),
            ));
        }
        let run_id = Uuid::new_v4();
        let dispatch_sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(dispatch_sequence), 0) + 1
             FROM mailbox_deliveries WHERE instance_id = $1",
        )
        .bind(target.instance_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        let updated = sqlx::query(
            "UPDATE mailbox_deliveries SET disposition = 'leased', logical_attempt_count = $2,
                    dispatch_sequence = $3, updated_at = now()
             WHERE event_id = $1 AND disposition = 'eligible'",
        )
        .bind(command.event_id.as_uuid())
        .bind(attempt_number)
        .bind(dispatch_sequence)
        .execute(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        if updated.rows_affected() != 1 {
            transaction.commit().await.map_err(dispatch_error)?;
            return Ok(None);
        }
        let attempt_id = MailboxOperationIdentity::attempt(
            MailboxId::from_uuid(target.mailbox_id),
            command.event_id,
            attempt_number_u32,
        )
        .id();
        sqlx::query(
            "INSERT INTO runs (id, instance_id, instance_revision_id, release_id,
                 release_agent_id, attachment_id, run_kind, command_id, state,
                 requires_state, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7, 'queued', $8, now(), now())",
        )
        .bind(run_id)
        .bind(target.instance_id)
        .bind(target.instance_revision_id)
        .bind(target.release_id)
        .bind(target.release_agent_id)
        .bind(target.attachment_id)
        .bind(attempt_id.as_uuid())
        .bind(target.requires_state)
        .execute(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        sqlx::query(
            "INSERT INTO mailbox_delivery_attempts
              (id, event_id, mailbox_id, attempt_number, state, command_id, instance_id,
               instance_revision_id, run_id, state_access_outcome)
             VALUES ($1, $2, $3, $4, 'leased', $5, $6, $7, $8, $9)",
        )
        .bind(attempt_id.as_uuid())
        .bind(command.event_id.as_uuid())
        .bind(target.mailbox_id)
        .bind(attempt_number)
        .bind(attempt_id.as_uuid())
        .bind(target.instance_id)
        .bind(target.instance_revision_id)
        .bind(run_id)
        .bind(if target.requires_state {
            "uncertain_access"
        } else {
            "no_state"
        })
        .execute(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        transaction.commit().await.map_err(dispatch_error)?;
        Ok(Some(StartRun {
            command_id: CommandId::from_uuid(attempt_id.as_uuid()),
            run_id: RunId::from_uuid(run_id),
            instance_id: AgentInstanceId::from_uuid(target.instance_id),
            instance_revision_id: AgentInstanceRevisionId::from_uuid(target.instance_revision_id),
            release_id: ReleaseId::from_uuid(target.release_id),
            release_agent_id: ReleaseAgentId::from_uuid(target.release_agent_id),
            attachment_id: Some(AgentAttachmentId::from_uuid(target.attachment_id)),
            kind: RunKind::Normal,
            requires_state: target.requires_state,
        }))
    }

    async fn record_run_resources(&self, run: &Run) -> Result<(), MailboxDispatchStoreError> {
        let mut transaction = worker_transaction(&self.pool).await?;
        let is_mailbox_run = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM mailbox_delivery_attempts WHERE run_id = $1)",
        )
        .bind(run.id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        // This observer is installed on the shared run orchestrator. Normal
        // runs not claimed from a mailbox have no delivery evidence to record.
        if !is_mailbox_run {
            transaction.commit().await.map_err(dispatch_error)?;
            return Ok(());
        }
        let snapshot_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM run_authorization_snapshots WHERE run_id = $1",
        )
        .bind(run.id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(dispatch_error)?
        .ok_or_else(|| {
            MailboxDispatchStoreError(
                "mailbox run has no durable authorization snapshot".to_owned(),
            )
        })?;
        sqlx::query(
            "UPDATE mailbox_delivery_attempts SET state = 'running', authorization_snapshot_id = $2,
                    state_volume_id = $3, lease_id = $4, lease_fencing_token = $5
             WHERE run_id = $1 AND state IN ('leased', 'running')",
        )
        .bind(run.id.as_uuid())
        .bind(snapshot_id)
        .bind(run.volume_id.map(runtime_types::VolumeId::as_uuid))
        .bind(run.lease_id.map(runtime_types::LeaseId::as_uuid))
        .bind(run.lease_fencing_token)
        .execute(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        transaction.commit().await.map_err(dispatch_error)
    }

    async fn settle_run(&self, run: &Run) -> Result<(), MailboxDispatchStoreError> {
        if run.state != RunState::CleanedUp {
            return Err(MailboxDispatchStoreError(
                "run has not cleaned up".to_owned(),
            ));
        }
        let mut transaction = worker_transaction(&self.pool).await?;
        let row = sqlx::query_as::<_, (Uuid, Uuid, i32)>(
            "SELECT event_id, mailbox_id, attempt_number FROM mailbox_delivery_attempts
             WHERE run_id = $1 FOR UPDATE",
        )
        .bind(run.id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        let Some((event_id, mailbox_id, attempt_number)) = row else {
            transaction.commit().await.map_err(dispatch_error)?;
            return Ok(());
        };
        let success = run.outcome == Some(RunOutcome::Succeeded);
        let terminal = !success && attempt_number >= 100;
        sqlx::query(
            "UPDATE mailbox_delivery_attempts SET state = $2, completed_at = now(),
                    state_access_outcome = CASE WHEN $3 THEN 'completed_access' ELSE state_access_outcome END
             WHERE run_id = $1 AND state NOT IN ('completed', 'failed', 'uncertain')",
        )
        .bind(run.id.as_uuid())
        .bind(if success { "completed" } else { "failed" })
        .bind(success)
        .execute(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        if success || terminal {
            sqlx::query(
                "UPDATE mailbox_deliveries SET disposition = $2, terminal_at = now(), updated_at = now()
                 WHERE event_id = $1 AND disposition IN ('leased', 'running')",
            )
            .bind(event_id)
            .bind(if success { "delivered" } else { "dead_lettered" })
            .execute(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
        } else {
            sqlx::query(
                "UPDATE mailbox_deliveries SET disposition = 'retryable',
                     next_eligible_at = now() + make_interval(secs => LEAST(3600, 2 ^ logical_attempt_count)),
                     updated_at = now() WHERE event_id = $1 AND disposition IN ('leased', 'running')",
            )
            .bind(event_id)
            .execute(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
            let attempt_number = u32::try_from(attempt_number).map_err(dispatch_error)?;
            let retry = MailboxOperationIdentity::retry(
                MailboxId::from_uuid(mailbox_id),
                MailboxEventId::from_uuid(event_id),
                attempt_number,
            )
            .id();
            enqueue_command(
                &mut transaction,
                retry.as_uuid(),
                event_id,
                MAILBOX_RETRY_SUBJECT,
                "mailbox.retry.v1",
            )
            .await?;
        }
        transaction.commit().await.map_err(dispatch_error)
    }

    async fn recover(&self) -> Result<usize, MailboxDispatchStoreError> {
        let mut transaction = worker_transaction(&self.pool).await?;
        // A cleaned run has a durable outcome.  Reconcile it as that outcome
        // before considering a delivery abandoned; retrying a successfully
        // cleaned run would duplicate the application effect.
        let completed = sqlx::query_as::<_, (Uuid, Uuid, i32, Uuid, Option<String>)>(
            "SELECT delivery.event_id, delivery.mailbox_id, attempt.attempt_number,
                    attempt.run_id, run.outcome
             FROM mailbox_deliveries AS delivery
             JOIN mailbox_delivery_attempts AS attempt
               ON attempt.event_id = delivery.event_id
              AND attempt.attempt_number = delivery.logical_attempt_count
             JOIN runs AS run ON run.id = attempt.run_id
             WHERE delivery.disposition IN ('leased', 'running')
               AND run.state = 'cleaned_up'
             FOR UPDATE OF delivery, attempt",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        for (event_id, mailbox_id, attempt_number, run_id, outcome) in completed {
            let success = outcome.as_deref() == Some("succeeded");
            let terminal = !success && attempt_number >= 100;
            sqlx::query(
                "UPDATE mailbox_delivery_attempts
                 SET state = $2, completed_at = COALESCE(completed_at, now()),
                     state_access_outcome = CASE
                         WHEN $3 THEN 'completed_access'
                         WHEN state_access_outcome = 'uncertain_access' THEN 'failed_access'
                         ELSE state_access_outcome
                     END
                 WHERE run_id = $1 AND state NOT IN ('completed', 'failed', 'uncertain')",
            )
            .bind(run_id)
            .bind(if success { "completed" } else { "failed" })
            .bind(success)
            .execute(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
            if success || terminal {
                sqlx::query(
                    "UPDATE mailbox_deliveries SET disposition = $2, terminal_at = now(), updated_at = now()
                     WHERE event_id = $1 AND disposition IN ('leased', 'running')",
                )
                .bind(event_id)
                .bind(if success { "delivered" } else { "dead_lettered" })
                .execute(&mut *transaction)
                .await
                .map_err(dispatch_error)?;
            } else {
                sqlx::query(
                    "UPDATE mailbox_deliveries SET disposition = 'retryable',
                         next_eligible_at = now(), updated_at = now()
                     WHERE event_id = $1 AND disposition IN ('leased', 'running')",
                )
                .bind(event_id)
                .execute(&mut *transaction)
                .await
                .map_err(dispatch_error)?;
                schedule_retry(&mut transaction, event_id, mailbox_id, attempt_number).await?;
            }
        }
        let rows = sqlx::query_as::<_, (Uuid, Uuid, i32)>(
            "UPDATE mailbox_deliveries AS delivery SET disposition = 'retryable',
                 next_eligible_at = now(), updated_at = now()
             WHERE delivery.disposition IN ('leased', 'running')
               AND NOT EXISTS (
                    SELECT 1 FROM mailbox_delivery_attempts AS attempt
                    JOIN runs AS run ON run.id = attempt.run_id
                    WHERE attempt.event_id = delivery.event_id
                      AND attempt.attempt_number = delivery.logical_attempt_count
                      AND run.state <> 'cleaned_up'
               )
             RETURNING delivery.event_id, delivery.mailbox_id, delivery.logical_attempt_count",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        for (event_id, mailbox_id, attempt_number) in &rows {
            // No durable completed run is available for this claim. Retain
            // that uncertainty on the exact attempt before it is retried.
            sqlx::query(
                "UPDATE mailbox_delivery_attempts SET state = 'uncertain', completed_at = now()
                 WHERE event_id = $1 AND attempt_number = $2
                   AND state IN ('leased', 'running')",
            )
            .bind(*event_id)
            .bind(*attempt_number)
            .execute(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
            schedule_retry(&mut transaction, *event_id, *mailbox_id, *attempt_number).await?;
        }
        transaction.commit().await.map_err(dispatch_error)?;
        Ok(rows.len())
    }
}

async fn worker_transaction(
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
async fn take_outbox_claim(
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

async fn enqueue_command(
    transaction: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    event_id: Uuid,
    subject: &str,
    event_type: &str,
) -> Result<(), MailboxDispatchStoreError> {
    sqlx::query(
        "INSERT INTO outbox (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES ($1, 'mailbox_event', $2, $3, $4,
                 jsonb_build_object('operation_id', $1, 'mailbox_event_id', $2), now())
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(operation_id)
    .bind(event_id)
    .bind(subject)
    .bind(event_type)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(dispatch_error)
}

/// Schedules the deterministic wake-up for the next attempt in the current
/// worker transaction. The delivery must already be in `retryable` state.
async fn schedule_retry(
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
}

fn dispatch_error(error: impl std::fmt::Display) -> MailboxDispatchStoreError {
    MailboxDispatchStoreError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{MailboxPersistenceError, validate_payload};
    use mailbox_domain::{BodyReference, BodyReferenceId, ContentMetadata};
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    fn content(body: &[u8], encoding: Option<&str>) -> ContentMetadata {
        ContentMetadata::new(
            BodyReference::new(
                BodyReferenceId::from_uuid(Uuid::new_v4()),
                u32::try_from(body.len()).expect("bounded test body"),
                Sha256::digest(body).into(),
            )
            .expect("bounded body reference"),
            Some(String::from("application/octet-stream")),
            encoding.map(str::to_owned),
        )
        .expect("valid content metadata")
    }

    #[test]
    fn accepts_empty_identity_payload() {
        let content = content(&[], Some("identity"));
        assert!(validate_payload(&content, &[], 0).is_ok());
    }

    #[test]
    fn rejects_mismatched_payload_evidence_before_persistence() {
        let content = content(b"expected", None);
        assert!(matches!(
            validate_payload(&content, b"different", 9),
            Err(MailboxPersistenceError::PayloadIntegrity)
        ));
        assert!(matches!(
            validate_payload(&content, b"expected", 7),
            Err(MailboxPersistenceError::PayloadIntegrity)
        ));
    }

    #[test]
    fn rejects_non_identity_content_encoding_before_persistence() {
        let content = content(b"body", Some("gzip"));
        assert!(matches!(
            validate_payload(&content, b"body", 4),
            Err(MailboxPersistenceError::UnsupportedContentEncoding)
        ));
    }
}

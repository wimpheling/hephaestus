use async_trait::async_trait;
use mailbox_dispatch::{
    MAILBOX_CANCEL_SUBJECT, MAILBOX_DISPATCH_SUBJECT, MAILBOX_RECOVERY_SUBJECT,
    MAILBOX_RETRY_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchCommand, MailboxDispatchStore,
    MailboxDispatchStoreError, MailboxOutboxRecord,
};
use mailbox_domain::MailboxEventId;
use run_domain::{Run, StartRun};
use uuid::Uuid;

use crate::{
    PostgresMailboxRepository,
    errors::dispatch_error,
    helpers::{OutboxRow, enqueue_next_dispatch, take_outbox_claim, worker_transaction},
};
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
                WHERE subject = $1
                  AND payload->>'operation_id' = $2::text
                  AND payload->>'mailbox_event_id' = $3::text
             )",
        )
        .bind(subject)
        .bind(command.operation_id.as_uuid())
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
            enqueue_next_dispatch(
                &mut transaction,
                mailbox_id,
                command.event_id,
                command.operation_id.as_uuid(),
            )
            .await?;
        }
        transaction.commit().await.map_err(dispatch_error)
    }

    async fn claim_dispatch(
        &self,
        command: &MailboxDispatchCommand,
    ) -> Result<Option<StartRun>, MailboxDispatchStoreError> {
        self.claim_dispatch_impl(command).await
    }

    async fn record_run_resources(&self, run: &Run) -> Result<(), MailboxDispatchStoreError> {
        self.record_run_resources_impl(run).await
    }

    async fn settle_run(&self, run: &Run) -> Result<(), MailboxDispatchStoreError> {
        self.settle_run_impl(run).await
    }

    async fn recover(&self) -> Result<usize, MailboxDispatchStoreError> {
        self.recover_impl().await
    }

    async fn cleanup_expired_payloads(
        &self,
        limit: i64,
    ) -> Result<usize, MailboxDispatchStoreError> {
        self.cleanup_expired_payloads_impl(limit).await
    }
}

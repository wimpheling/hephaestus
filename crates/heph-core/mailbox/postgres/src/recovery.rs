use uuid::Uuid;

use crate::{
    PostgresMailboxRepository,
    errors::dispatch_error,
    helpers::{schedule_retry, worker_transaction},
};
use mailbox_dispatch::MailboxDispatchStoreError;
impl PostgresMailboxRepository {
    // Recovery deliberately keeps cleaned-outcome settlement, abandoned-claim
    // classification, and retry-command creation in one transaction; splitting
    // it would make a crash observable between those authoritative decisions.
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::redundant_pub_crate)]
    // This crate-private implementation is delegated by the single trait impl.
    pub(crate) async fn recover_impl(&self) -> Result<usize, MailboxDispatchStoreError> {
        let mut transaction = worker_transaction(&self.pool).await?;
        // A cleaned run has a durable outcome.  Reconcile it as that outcome
        // before considering a delivery abandoned; retrying a successfully
        // cleaned run would duplicate the application effect.
        let completed =
            sqlx::query_as::<_, (Uuid, Uuid, i32, Uuid, Option<String>, Option<String>)>(
                "SELECT delivery.event_id, delivery.mailbox_id, attempt.attempt_number,
                    attempt.run_id, run.outcome, run.failure
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
        for (event_id, mailbox_id, attempt_number, run_id, outcome, failure) in completed {
            let success = outcome.as_deref() == Some("succeeded");
            let authorization_denied = failure.as_deref().is_some_and(|failure| {
                failure.starts_with("run launch authorization failed:")
                    || failure.starts_with("run authority operation failed:")
            });
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
            if success || terminal || authorization_denied {
                sqlx::query(
                    "UPDATE mailbox_deliveries SET disposition = $2, terminal_at = now(),
                         denial_code = $3, updated_at = now()
                     WHERE event_id = $1 AND disposition IN ('leased', 'running')",
                )
                .bind(event_id)
                .bind(if success {
                    "delivered"
                } else if authorization_denied {
                    "denied"
                } else {
                    "dead_lettered"
                })
                .bind(authorization_denied.then_some("runtime_authorization_denied"))
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
        // The retry command must not be published before its durable backoff
        // expires: a JetStream consumer would acknowledge an early no-op and
        // leave the delivery stranded. The recovery loop owns that due-time
        // transition and its deterministic outbox command.
        let due_retries = sqlx::query_as::<_, (Uuid, Uuid, i32)>(
            "SELECT event_id, mailbox_id, logical_attempt_count
             FROM mailbox_deliveries
             WHERE disposition = 'retryable' AND next_eligible_at <= now()",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        for (event_id, mailbox_id, attempt_number) in due_retries {
            schedule_retry(&mut transaction, event_id, mailbox_id, attempt_number).await?;
        }
        let rows = sqlx::query_as::<_, (Uuid, Uuid, i32)>(
            "UPDATE mailbox_deliveries AS delivery SET disposition = 'retryable',
                 next_eligible_at = now(), updated_at = now()
             WHERE delivery.disposition IN ('leased', 'running')
               -- A separate recovery statement can start before cleanup commits
               -- and reach this update afterwards under READ COMMITTED.  Any
               -- matching run is authoritative, including one that became
               -- cleaned after the first reconciliation statement's snapshot;
               -- leave it for the next recovery pass instead of marking the
               -- attempt uncertain.
               AND NOT EXISTS (
                    SELECT 1 FROM mailbox_delivery_attempts AS attempt
                    JOIN runs AS run ON run.id = attempt.run_id
                    WHERE attempt.event_id = delivery.event_id
                      AND attempt.attempt_number = delivery.logical_attempt_count
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

    #[allow(clippy::redundant_pub_crate)]
    // This crate-private implementation is delegated by the single trait impl.
    pub(crate) async fn cleanup_expired_payloads_impl(
        &self,
        limit: i64,
    ) -> Result<usize, MailboxDispatchStoreError> {
        let limit = i32::try_from(limit.clamp(1, 1_000)).map_err(dispatch_error)?;
        let mut transaction = worker_transaction(&self.pool).await?;
        let purged: i32 = sqlx::query_scalar("SELECT purge_expired_mailbox_payloads($1)")
            .bind(limit)
            .fetch_one(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
        transaction.commit().await.map_err(dispatch_error)?;
        usize::try_from(purged).map_err(dispatch_error)
    }
}

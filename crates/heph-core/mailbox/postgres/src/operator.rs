use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use mailbox_dispatch::{MAILBOX_CANCEL_SUBJECT, MAILBOX_RETRY_SUBJECT, MAILBOX_WAKE_SUBJECT};
use mailbox_domain::{MailboxEventId, MailboxId};
use uuid::Uuid;

use crate::{
    MailboxOperatorAction, MailboxOperatorResult, MailboxPersistenceError,
    PostgresMailboxRepository, errors::storage, helpers::enqueue_command,
};
impl PostgresMailboxRepository {
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
}

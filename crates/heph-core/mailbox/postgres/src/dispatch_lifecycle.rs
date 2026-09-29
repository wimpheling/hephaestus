use run_domain::{Run, RunOutcome, RunState};
use uuid::Uuid;

use crate::{PostgresMailboxRepository, errors::dispatch_error, helpers::worker_transaction};
use mailbox_dispatch::MailboxDispatchStoreError;
impl PostgresMailboxRepository {
    #[allow(clippy::redundant_pub_crate)]
    // This crate-private implementation is delegated by the single trait impl.
    pub(crate) async fn record_run_resources_impl(
        &self,
        run: &Run,
    ) -> Result<(), MailboxDispatchStoreError> {
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

    #[allow(clippy::redundant_pub_crate)]
    // This crate-private implementation is delegated by the single trait impl.
    pub(crate) async fn settle_run_impl(&self, run: &Run) -> Result<(), MailboxDispatchStoreError> {
        if run.state != RunState::CleanedUp {
            return Err(MailboxDispatchStoreError(
                "run has not cleaned up".to_owned(),
            ));
        }
        let mut transaction = worker_transaction(&self.pool).await?;
        // Keep the same delivery-then-attempt lock order as claiming and
        // recovery. A cleanup observer can race the recovery sweep after a
        // worker crash; locking the attempt first here previously inverted
        // that order and let PostgreSQL detect a deadlock.
        let row = sqlx::query_as::<_, (Uuid, Uuid, i32)>(
            "SELECT delivery.event_id, delivery.mailbox_id, attempt.attempt_number
               FROM mailbox_deliveries AS delivery
               JOIN mailbox_delivery_attempts AS attempt
                 ON attempt.event_id = delivery.event_id
              WHERE attempt.run_id = $1
              FOR UPDATE OF delivery, attempt",
        )
        .bind(run.id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        let Some((event_id, _mailbox_id, attempt_number)) = row else {
            transaction.commit().await.map_err(dispatch_error)?;
            return Ok(());
        };
        let success = run.outcome == Some(RunOutcome::Succeeded);
        // Live launch and runtime-authority checks happen immediately before
        // provisioning. A failure there is a durable authorization decision,
        // not a transient guest failure to retry. Keep one stable, redacted
        // disposition so operators can distinguish it from application work.
        let authorization_denied = run.failure.as_deref().is_some_and(|failure| {
            failure.starts_with("run launch authorization failed:")
                || failure.starts_with("run authority operation failed:")
        });
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
                     next_eligible_at = now() + make_interval(secs => LEAST(3600, 2 ^ logical_attempt_count)),
                     updated_at = now() WHERE event_id = $1 AND disposition IN ('leased', 'running')",
            )
            .bind(event_id)
            .execute(&mut *transaction)
            .await
            .map_err(dispatch_error)?;
        }
        transaction.commit().await.map_err(dispatch_error)
    }
}

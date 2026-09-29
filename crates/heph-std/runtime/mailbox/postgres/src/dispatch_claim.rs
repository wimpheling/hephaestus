use mailbox_domain::{MailboxId, MailboxOperationIdentity};
use run_domain::{RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use uuid::Uuid;

use crate::{
    PostgresMailboxRepository,
    errors::dispatch_error,
    helpers::{DispatchTargetRow, worker_transaction},
};
use mailbox_dispatch::{MailboxDispatchCommand, MailboxDispatchStoreError};
impl PostgresMailboxRepository {
    // One transaction deliberately keeps the eligibility recheck, per-instance
    // serialization, attempt, and pre-created run adjacent for auditability.
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::redundant_pub_crate)]
    // This crate-private implementation is delegated by the single trait impl.
    pub(crate) async fn claim_dispatch_impl(
        &self,
        command: &MailboxDispatchCommand,
    ) -> Result<Option<StartRun>, MailboxDispatchStoreError> {
        let mut transaction = worker_transaction(&self.pool).await?;
        let target = sqlx::query_as::<_, DispatchTargetRow>(
            "SELECT event.mailbox_id, event.instance_id, revision.id AS instance_revision_id,
                    release_agent.release_id, revision.release_agent_id,
                    attachment.id AS attachment_id, git_ref.git_ref AS target_ref,
                    git_ref.commit_sha AS target_commit, release_agent.requires_state,
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
                SELECT id, repository_id, ref_selector FROM agent_attachments
                WHERE instance_id = instance.id AND enabled AND removed_at IS NULL
                ORDER BY created_at, id LIMIT 1
             ) AS attachment ON true
             JOIN git_refs AS git_ref
               ON git_ref.repository_id = attachment.repository_id
              AND git_ref.git_ref = attachment.ref_selector
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
        // The first target read establishes which instance-scoped advisory
        // lock to wait on. Re-read the target after that wait so a concurrent
        // gate/revision change cannot be bypassed by the old statement
        // snapshot.
        let target = sqlx::query_as::<_, DispatchTargetRow>(
            "SELECT event.mailbox_id, event.instance_id, revision.id AS instance_revision_id,
                    release_agent.release_id, revision.release_agent_id,
                    attachment.id AS attachment_id, git_ref.git_ref AS target_ref,
                    git_ref.commit_sha AS target_commit, release_agent.requires_state,
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
                SELECT id, repository_id, ref_selector FROM agent_attachments
                WHERE instance_id = instance.id AND enabled AND removed_at IS NULL
                ORDER BY created_at, id LIMIT 1
             ) AS attachment ON true
             JOIN git_refs AS git_ref
               ON git_ref.repository_id = attachment.repository_id
              AND git_ref.git_ref = attachment.ref_selector
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
        let instance_busy: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM runs
                  WHERE instance_id = $1 AND requires_state AND state <> 'cleaned_up'
             ) AND $2",
        )
        .bind(target.instance_id)
        .bind(target.requires_state)
        .fetch_one(&mut *transaction)
        .await
        .map_err(dispatch_error)?;
        if instance_busy {
            // Leave the committed dispatch command and eligible delivery
            // untouched. The transport handler leaves this error unacknowledged
            // so JetStream redelivers the same identifier-only command after
            // its acknowledgement deadline, without creating another attempt.
            return Err(MailboxDispatchStoreError(
                "mailbox instance has an active stateful run".to_owned(),
            ));
        }
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
               instance_revision_id, run_id, target_ref, target_commit, state_access_outcome)
             VALUES ($1, $2, $3, $4, 'leased', $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(attempt_id.as_uuid())
        .bind(command.event_id.as_uuid())
        .bind(target.mailbox_id)
        .bind(attempt_number)
        .bind(attempt_id.as_uuid())
        .bind(target.instance_id)
        .bind(target.instance_revision_id)
        .bind(run_id)
        .bind(&target.target_ref)
        .bind(&target.target_commit)
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
}

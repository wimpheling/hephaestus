//! Update hook admission command.

use super::{
    AgentInstanceId, AgentInstanceRevisionId, AuthenticatedIdentity, BeginUpdateHook, CommandId,
    ObjectRef, ObjectType, Permission, RUN_START_SUBJECT, ReleaseAgentId, ReleaseId,
    ReleaseService, ReleaseServiceError, RunKind, StartRun, UpdateHookAdmissionRow, append_event,
    begin_actor_transaction, existing_command, is_update_admission_generation_conflict,
    record_command,
};
use serde_json::json;

impl ReleaseService {
    /// Enters the isolated hook only after all pre-gate normal work drains and
    /// creates the exact queued update run. The run orchestrator acquires the
    /// optional state volume under its normal fenced exclusive lease.
    ///
    /// # Errors
    ///
    /// Fails for denial, undrained work, stale lifecycle, idempotency conflict,
    /// or database failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            update_id = %command.update_id,
            run_id = %command.hook_run_id
        )
    )]
    pub async fn begin_update_hook(
        &self,
        identity: &AuthenticatedIdentity,
        command: BeginUpdateHook,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let update: UpdateHookAdmissionRow = sqlx::query_as(
            "SELECT update.instance_id, update.candidate_revision_id, update.state,
                    instance.state AS instance_state,
                    update.created_at,
                    candidate.release_agent_id,
                    release_agent.release_id,
                    release_agent.requires_state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             JOIN agent_instance_revisions AS candidate
               ON candidate.id = update.candidate_revision_id
              AND candidate.instance_id = update.instance_id
             JOIN release_agents AS release_agent
               ON release_agent.id = candidate.release_agent_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(command.update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUpdate,
            ObjectRef::new(ObjectType::AgentInstance, update.instance_id),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "begin_update_hook")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        if update.state != "draining" || update.instance_state != "update_draining" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM run_requests
                 WHERE instance_id = $1
                   AND request_kind = 'instance_normal'
                   AND dispatch_state = 'pending'
                   AND created_at <= $2
             ) OR EXISTS(
                 SELECT 1
                 FROM run_instance_provenance AS provenance
                 JOIN runs ON runs.id = provenance.run_id
                 WHERE provenance.instance_id = $1
                   AND provenance.phase = 'normal'
                   AND runs.state <> 'cleaned_up'
             ) OR EXISTS(
                 SELECT 1
                 FROM runs
                 WHERE runs.instance_id = $1
                   AND runs.run_kind = 'normal'
                   AND runs.state <> 'cleaned_up'
             )",
        )
        .bind(update.instance_id)
        .bind(update.created_at)
        .fetch_one(&mut *tx)
        .await?;
        if pending {
            return Err(ReleaseServiceError::UpdateDrainPending);
        }
        let start_id = CommandId::new();
        let insert_result = sqlx::query(
            "INSERT INTO runs
             (id, instance_id, instance_revision_id, release_id,
              release_agent_id, run_kind, command_id, state, requires_state,
              created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, 'update', $6, 'queued', $7,
                     now(), now())",
        )
        .bind(command.hook_run_id.as_uuid())
        .bind(update.instance_id)
        .bind(update.candidate_revision_id)
        .bind(update.release_id)
        .bind(update.release_agent_id)
        .bind(start_id.as_uuid())
        .bind(update.requires_state)
        .execute(&mut *tx)
        .await;
        match insert_result {
            Ok(_) => {}
            Err(error) if is_update_admission_generation_conflict(&error) => {
                return Err(ReleaseServiceError::UpdateAdmissionGenerationRace);
            }
            Err(error) => return Err(error.into()),
        }
        let changed = sqlx::query(
            "UPDATE agent_updates
             SET state = 'hook_running', hook_run_id = $2, updated_at = now()
             WHERE id = $1 AND state = 'draining'",
        )
        .bind(command.update_id.as_uuid())
        .bind(command.hook_run_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        sqlx::query(
            "UPDATE agent_instances
             SET state = 'updating', version = version + 1, updated_at = now()
             WHERE id = $1 AND state = 'update_draining'
               AND NOT run_gate_open",
        )
        .bind(update.instance_id)
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "begin_update_hook",
            command.update_id.as_uuid(),
            Some(command.hook_run_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            command.update_id.as_uuid(),
            "hephaestus.agent_update.hook_started.v1",
            "agent_update.hook_started.v1",
            json!({
                "schema_version": 1,
                "update_id": command.update_id,
                "hook_run_id": command.hook_run_id,
            }),
        )
        .await?;
        let start = StartRun {
            command_id: start_id,
            run_id: command.hook_run_id,
            instance_id: AgentInstanceId::from_uuid(update.instance_id),
            instance_revision_id: AgentInstanceRevisionId::from_uuid(update.candidate_revision_id),
            release_id: ReleaseId::from_uuid(update.release_id),
            release_agent_id: ReleaseAgentId::from_uuid(update.release_agent_id),
            attachment_id: None,
            kind: RunKind::Update,
            requires_state: update.requires_state,
        };
        append_event(
            &mut tx,
            command.hook_run_id.as_uuid(),
            RUN_START_SUBJECT,
            "run.start.v1",
            serde_json::to_value(start)?,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

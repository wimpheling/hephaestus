use super::release_command_events::append_event;
use super::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, CommandId,
    DeferredMaterializationRow, MAILBOX_WAKE_EVENT_TYPE, MAILBOX_WAKE_SUBJECT, Postgres,
    RUN_START_SUBJECT, ReleaseAgentId, ReleaseId, ReleaseServiceError, RunId, RunKind, StartRun,
    Transaction, UpdateDecision, UpdateRecoveryAction, UpdateRecoveryDecision, Uuid, json,
};

pub fn existing_terminal_decision(state: &str) -> Result<UpdateDecision, ReleaseServiceError> {
    match state {
        "activated" => Ok(UpdateDecision::Activated),
        "rejected" => Ok(UpdateDecision::AgentRejected),
        "compatibility_unknown" => Ok(UpdateDecision::CompatibilityUnknown),
        "hook_committed" | "activation_recovery" => Ok(UpdateDecision::ActivationRecovery),
        _ => Err(ReleaseServiceError::InvalidUpdateLifecycle),
    }
}

pub const fn recovery_decision(action: UpdateRecoveryAction) -> UpdateRecoveryDecision {
    match action {
        UpdateRecoveryAction::RetryHook => UpdateRecoveryDecision::HookRetryScheduled,
        UpdateRecoveryAction::RejectCandidate => UpdateRecoveryDecision::CandidateRejected,
        UpdateRecoveryAction::ResumeActivation => UpdateRecoveryDecision::CandidateActivated,
    }
}

pub async fn mark_volume_lease(
    tx: &mut Transaction<'_, Postgres>,
    update_id: AgentUpdateId,
    state: &str,
) -> Result<(), ReleaseServiceError> {
    let volume_id: Option<Uuid> = sqlx::query_scalar(
        "UPDATE agent_instance_volume_leases
         SET state = $2,
             released_at = CASE WHEN $2 = 'released' THEN now() END
         WHERE update_id = $1
           AND (
               state = 'active'
               OR ($2 = 'released' AND state = 'recovery_required')
           )
         RETURNING volume_id",
    )
    .bind(update_id.as_uuid())
    .bind(state)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(volume_id) = volume_id {
        sqlx::query(
            "UPDATE agent_instance_state_volumes
             SET state = CASE
                    WHEN $2 = 'released' THEN 'ready'
                    ELSE 'recovering'
                 END,
                 updated_at = now()
             WHERE id = $1",
        )
        .bind(volume_id)
        .bind(state)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn reopen_after_update(
    tx: &mut Transaction<'_, Postgres>,
    update_id: AgentUpdateId,
    instance_id: Uuid,
    revision_id: Uuid,
    state: &str,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "UPDATE agent_instances
         SET state = $3, run_gate_open = true, version = version + 1,
             updated_at = now()
         WHERE id = $1 AND active_revision_id = $2
           AND state IN ('updating', 'paused_unknown_state')
           AND NOT run_gate_open",
    )
    .bind(instance_id)
    .bind(revision_id)
    .bind(state)
    .execute(&mut **tx)
    .await?;
    mark_volume_lease(tx, update_id, "released").await?;
    materialize_deferred_triggers(tx, instance_id, revision_id).await?;
    enqueue_mailbox_wakes(tx, instance_id).await
}

/// Re-wakes accepted mailbox deliveries after an instance gate reopens.
///
/// A dispatch command can be consumed while an update has the instance gate
/// closed.  The delivery remains eligible in that case, but the consumed
/// command cannot be replayed after activation.  These identifier-only wake
/// records are committed with the gate transition, so the outbox publisher
/// supplies a new authoritative dispatch command without creating a run or
/// changing the delivery attempt count.
pub async fn enqueue_mailbox_wakes(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: Uuid,
) -> Result<(), ReleaseServiceError> {
    let event_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT event_id
         FROM mailbox_deliveries
         WHERE instance_id = $1
           AND (disposition IN ('pending', 'eligible')
                OR (disposition = 'retryable' AND next_eligible_at <= now()))
         ORDER BY updated_at, event_id",
    )
    .bind(instance_id)
    .fetch_all(&mut **tx)
    .await?;
    for event_id in event_ids {
        let operation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO outbox
             (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
             VALUES ($1, 'mailbox_event', $2, $3, $4,
                     jsonb_build_object(
                         'schema_version', 1,
                         'command_kind', 'wake',
                         'operation_id', $1,
                         'mailbox_event_id', $2
                     ), now())",
        )
        .bind(operation_id)
        .bind(event_id)
        .bind(MAILBOX_WAKE_SUBJECT)
        .bind(MAILBOX_WAKE_EVENT_TYPE)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn materialize_deferred_triggers(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: Uuid,
    revision_id: Uuid,
) -> Result<(), ReleaseServiceError> {
    let rows: Vec<DeferredMaterializationRow> = sqlx::query_as(
        "SELECT deferred.id, deferred.attachment_id,
                deferred.repository_id, deferred.target_ref,
                deferred.target_commit, deferred.source_id,
                release.id AS release_id, release_agent.id AS release_agent_id,
                revision.platform_policy_version, release_agent.requires_state
         FROM deferred_agent_triggers AS deferred
         JOIN agent_attachments AS attachment
           ON attachment.id = deferred.attachment_id
          AND attachment.instance_id = deferred.instance_id
         JOIN agent_instance_revisions AS revision
           ON revision.id = $2 AND revision.instance_id = deferred.instance_id
         JOIN release_agents AS release_agent
           ON release_agent.id = revision.release_agent_id
         JOIN releases AS release ON release.id = release_agent.release_id
         WHERE deferred.instance_id = $1 AND deferred.state = 'deferred'
           AND attachment.enabled AND attachment.removed_at IS NULL
           AND revision.runnable AND release.state = 'published'
         ORDER BY deferred.created_at, deferred.id",
    )
    .bind(instance_id)
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in rows {
        let request_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        let stored_request: Uuid = sqlx::query_scalar(
            "INSERT INTO run_requests
             (id, repository_id, commit_sha, git_ref, receive_id,
              run_id, command_id, instance_id, instance_revision_id,
              release_id, release_agent_id, attachment_id, request_kind,
              platform_policy_version, requires_state)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     $11, $12, 'instance_normal', $13, $14)
             ON CONFLICT (
                 attachment_id, instance_revision_id, commit_sha, git_ref,
                 receive_id, attempt
             ) WHERE request_kind = 'instance_normal'
             DO UPDATE SET repository_id = EXCLUDED.repository_id
             RETURNING id",
        )
        .bind(request_id)
        .bind(row.repository_id)
        .bind(&row.target_commit)
        .bind(&row.target_ref)
        .bind(row.source_id)
        .bind(run_id)
        .bind(command_id)
        .bind(instance_id)
        .bind(revision_id)
        .bind(row.release_id)
        .bind(row.release_agent_id)
        .bind(row.attachment_id)
        .bind(&row.platform_policy_version)
        .bind(row.requires_state)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE deferred_agent_triggers
             SET state = 'materialized', run_request_id = $2,
                 resolved_at = now()
             WHERE id = $1 AND state = 'deferred'",
        )
        .bind(row.id)
        .bind(stored_request)
        .execute(&mut **tx)
        .await?;
        append_event(
            tx,
            stored_request,
            "hephaestus.instance.run.requested.v1",
            "instance.run.requested.v1",
            json!({
                "schema_version": 1,
                "run_request_id": stored_request,
                "run_id": run_id,
                "command_id": command_id,
                "instance_id": instance_id,
                "instance_revision_id": revision_id,
                "release_id": row.release_id,
                "release_agent_id": row.release_agent_id,
                "attachment_id": row.attachment_id,
                "target_repository_id": row.repository_id,
                "target_ref": row.target_ref,
                "target_commit": row.target_commit,
                "requires_state": row.requires_state,
            }),
        )
        .await?;
        append_deferred_start(tx, &row, instance_id, revision_id, run_id, command_id).await?;
    }
    deny_unmaterializable_triggers(tx, instance_id).await
}

pub async fn append_deferred_start(
    tx: &mut Transaction<'_, Postgres>,
    row: &DeferredMaterializationRow,
    instance_id: Uuid,
    revision_id: Uuid,
    run_id: Uuid,
    command_id: Uuid,
) -> Result<(), ReleaseServiceError> {
    let start = StartRun {
        command_id: CommandId::from_uuid(command_id),
        run_id: RunId::from_uuid(run_id),
        instance_id: AgentInstanceId::from_uuid(instance_id),
        instance_revision_id: AgentInstanceRevisionId::from_uuid(revision_id),
        release_id: ReleaseId::from_uuid(row.release_id),
        release_agent_id: ReleaseAgentId::from_uuid(row.release_agent_id),
        attachment_id: Some(AgentAttachmentId::from_uuid(row.attachment_id)),
        kind: RunKind::Normal,
        requires_state: row.requires_state,
    };
    append_event(
        tx,
        run_id,
        RUN_START_SUBJECT,
        "run.start.v1",
        serde_json::to_value(start)?,
    )
    .await
}

pub async fn deny_unmaterializable_triggers(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: Uuid,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "UPDATE deferred_agent_triggers AS deferred
         SET state = 'denied',
             diagnostics = '[{\"code\":\"trigger_no_longer_authorized\"}]',
             resolved_at = now()
         WHERE deferred.instance_id = $1 AND deferred.state = 'deferred'
           AND NOT EXISTS (
               SELECT 1 FROM agent_attachments AS attachment
               WHERE attachment.id = deferred.attachment_id
                 AND attachment.enabled AND attachment.removed_at IS NULL
           )",
    )
    .bind(instance_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

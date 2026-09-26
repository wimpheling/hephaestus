use super::{
    helpers::{serialization, storage},
    outbox::append_outbox,
};
use crate::{INSTANCE_RUN_REQUESTED_SUBJECT, RUN_START_SUBJECT};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use forge_domain::{ReceiveId, RefUpdate, Repository, RunRequestId};
use forge_service::ForgeRepositoryError;
use identity_domain::AuthenticatedIdentity;
use run_domain::{RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct InstanceTriggerRow {
    attachment_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    platform_policy_version: String,
    run_gate_open: bool,
    instance_state: String,
    runnable: bool,
    requires_state: bool,
}

// This atomic trigger workflow keeps authorization, deduplication, and outbox writes together.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn persist_instance_triggers(
    transaction: &mut Transaction<'_, Postgres>,
    repository: &Repository,
    receive_id: ReceiveId,
    identity: Option<&AuthenticatedIdentity>,
    updates: &[RefUpdate],
    authorizer: Option<&PostgresMelangeAuthorizer>,
    now: OffsetDateTime,
    originating_attachment_id: Option<Uuid>,
    runtime_receive: bool,
) -> Result<(), ForgeRepositoryError> {
    if !repository.agent_runs_enabled {
        return Ok(());
    }
    for update in updates {
        let Some(commit) = &update.new_commit else {
            continue;
        };
        let candidates: Vec<InstanceTriggerRow> = sqlx::query_as(
            "SELECT attachment.id AS attachment_id,
                    instance.id AS instance_id,
                    revision.id AS instance_revision_id,
                    release.id AS release_id,
                    release_agent.id AS release_agent_id,
                    revision.platform_policy_version,
                    instance.run_gate_open, instance.state AS instance_state,
                    revision.runnable
                    , release_agent.requires_state
             FROM agent_attachments AS attachment
             JOIN agent_instances AS instance
               ON instance.id = attachment.instance_id
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id
             JOIN release_agents AS release_agent
               ON release_agent.id = revision.release_agent_id
             JOIN releases AS release ON release.id = release_agent.release_id
             WHERE attachment.repository_id = $1
               AND ($3::uuid IS NULL OR attachment.id <> $3)
               AND attachment.enabled AND attachment.removed_at IS NULL
               AND attachment.trigger_policy IN ('push', 'push_and_manual')
               AND release.state = 'published'
               AND (
                   attachment.ref_selector = $2
                   OR (
                       right(attachment.ref_selector, 2) = '/*'
                       AND $2 LIKE
                           left(
                               attachment.ref_selector,
                               length(attachment.ref_selector) - 1
                           ) || '%'
                   )
               )
             ORDER BY attachment.id",
        )
        .bind(repository.id.as_uuid())
        .bind(update.git_ref.as_str())
        .bind(originating_attachment_id)
        .fetch_all(&mut **transaction)
        .await
        .map_err(storage)?;
        for candidate in candidates {
            if runtime_receive {
                // Runtime authorization snapshots currently expose workload,
                // repository, and state-volume capabilities. They cannot
                // prove human CanExecute/CanUse decisions for sibling
                // attachments, so fail closed instead of inventing an actor.
                continue;
            }
            let mut permitted = true;
            if let (Some(identity), Some(authorizer)) = (identity, authorizer) {
                for (permission, object) in [
                    (
                        Permission::CanExecute,
                        ObjectRef::new(ObjectType::AgentAttachment, candidate.attachment_id),
                    ),
                    (
                        Permission::CanUse,
                        ObjectRef::new(ObjectType::ReleaseAgent, candidate.release_agent_id),
                    ),
                ] {
                    let decision = authorizer
                        .check(
                            transaction,
                            Subject::User(identity.user_id),
                            permission,
                            object,
                        )
                        .await
                        .map_err(storage)?;
                    audit_decision(
                        transaction,
                        identity.user_id,
                        permission,
                        object,
                        decision,
                        identity.request_id,
                    )
                    .await
                    .map_err(storage)?;
                    if decision == AuthorizationDecision::Deny {
                        permitted = false;
                        break;
                    }
                }
            }
            if !permitted {
                continue;
            }
            if !candidate.run_gate_open {
                sqlx::query(
                    "INSERT INTO deferred_agent_triggers
                     (id, instance_id, attachment_id, repository_id,
                      target_ref, target_commit, source_id)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (
                         attachment_id, repository_id, target_ref,
                         target_commit, source_id
                     ) DO NOTHING",
                )
                .bind(Uuid::new_v4())
                .bind(candidate.instance_id)
                .bind(candidate.attachment_id)
                .bind(repository.id.as_uuid())
                .bind(update.git_ref.as_str())
                .bind(commit.as_str())
                .bind(receive_id.as_uuid())
                .execute(&mut **transaction)
                .await
                .map_err(storage)?;
                continue;
            }
            if !candidate.runnable
                || !["active", "update_rejected"].contains(&candidate.instance_state.as_str())
            {
                continue;
            }
            let request_id = RunRequestId::new();
            let run_id = RunId::new();
            let command_id = CommandId::new();
            let stored: (Uuid, Uuid, Uuid) = sqlx::query_as(
                "INSERT INTO run_requests
                 (id, repository_id, commit_sha, git_ref, receive_id,
                  run_id, command_id, actor_id, request_id, created_at,
                  instance_id, instance_revision_id, release_id,
                  release_agent_id, attachment_id, request_kind,
                  platform_policy_version, requires_state)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                         $11, $12, $13, $14, $15, 'instance_normal', $16, $17)
                 ON CONFLICT (
                     attachment_id, instance_revision_id, commit_sha, git_ref,
                     receive_id, attempt
                 ) WHERE request_kind = 'instance_normal'
                 DO UPDATE SET repository_id = EXCLUDED.repository_id
                 RETURNING id, run_id, command_id",
            )
            .bind(request_id.as_uuid())
            .bind(repository.id.as_uuid())
            .bind(commit.as_str())
            .bind(update.git_ref.as_str())
            .bind(receive_id.as_uuid())
            .bind(run_id.as_uuid())
            .bind(command_id.as_uuid())
            .bind(identity.map(|value| value.user_id.as_uuid()))
            .bind(identity.map(|value| value.request_id.as_uuid()))
            .bind(now)
            .bind(candidate.instance_id)
            .bind(candidate.instance_revision_id)
            .bind(candidate.release_id)
            .bind(candidate.release_agent_id)
            .bind(candidate.attachment_id)
            .bind(&candidate.platform_policy_version)
            .bind(candidate.requires_state)
            .fetch_one(&mut **transaction)
            .await
            .map_err(storage)?;
            append_outbox(
                transaction,
                stored.0,
                INSTANCE_RUN_REQUESTED_SUBJECT,
                "instance.run.requested.v1",
                json!({
                    "schema_version": 1,
                    "run_request_id": stored.0,
                    "run_id": stored.1,
                    "command_id": stored.2,
                    "receive_id": receive_id,
                    "instance_id": candidate.instance_id,
                    "instance_revision_id": candidate.instance_revision_id,
                    "release_id": candidate.release_id,
                    "release_agent_id": candidate.release_agent_id,
                    "attachment_id": candidate.attachment_id,
                    "target_repository_id": repository.id,
                    "target_ref": update.git_ref,
                    "target_commit": commit,
                    "platform_policy_version": candidate.platform_policy_version,
                    "requires_state": candidate.requires_state,
                }),
                now,
            )
            .await?;
            let command = StartRun {
                command_id: CommandId::from_uuid(stored.2),
                run_id: RunId::from_uuid(stored.1),
                instance_id: AgentInstanceId::from_uuid(candidate.instance_id),
                instance_revision_id: AgentInstanceRevisionId::from_uuid(
                    candidate.instance_revision_id,
                ),
                release_id: ReleaseId::from_uuid(candidate.release_id),
                release_agent_id: ReleaseAgentId::from_uuid(candidate.release_agent_id),
                attachment_id: Some(AgentAttachmentId::from_uuid(candidate.attachment_id)),
                kind: RunKind::Normal,
                requires_state: candidate.requires_state,
            };
            append_outbox(
                transaction,
                stored.1,
                RUN_START_SUBJECT,
                "run.start.v1",
                serde_json::to_value(command).map_err(serialization)?,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

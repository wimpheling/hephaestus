//! Authorized run, retry, and review command mutations.

use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::PostgresMelangeAuthorizer;
use release_domain::{AgentInstanceRevisionId, ReleaseAgentId, ReleaseId};
use review_domain::ControlCommand;
use review_service::{ControlOutcome, ReviewRepositoryError};
use run_domain::{RunKind, StartRun};
use runtime_types::{AgentAttachmentId as RuntimeAttachmentId, AgentInstanceId, CommandId, RunId};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::models::{ProposalRow, RetrySource};
use super::support::{db_error, infrastructure};
use super::transaction::{
    append_run_event, authorize, close_denied, close_rejected, complete_control, insert_outbox,
};
use super::{CANCEL_RUN_SUBJECT, START_RUN_SUBJECT};

pub async fn cancel_run(
    authorizer: &PostgresMelangeAuthorizer,
    transaction: &mut Transaction<'_, Postgres>,
    command: &ControlCommand,
) -> Result<ControlOutcome, ReviewRepositoryError> {
    let run_id = command
        .run_id
        .ok_or(ReviewRepositoryError::DeliveryMismatch)?;
    let decision = authorize(
        authorizer,
        transaction,
        command,
        Permission::CanCancel,
        ObjectRef::new(ObjectType::Run, run_id.as_uuid()),
    )
    .await?;
    if !decision.is_allowed() {
        close_denied(transaction, command.command_id).await?;
        return Ok(ControlOutcome::Denied);
    }
    insert_outbox(
        transaction,
        "run",
        run_id.as_uuid(),
        CANCEL_RUN_SUBJECT,
        "run.cancel_requested",
        serde_json::json!({
            "command_id": CommandId::from_uuid(command.command_id.as_uuid()),
            "run_id": run_id,
            "reason": command.reason,
        }),
    )
    .await?;
    complete_control(transaction, command.command_id).await?;
    append_run_event(
        transaction,
        run_id,
        "review.cancel_requested",
        serde_json::json!({"actor_id": command.actor_id, "reason": command.reason}),
    )
    .await?;
    Ok(ControlOutcome::Completed)
}

// This transaction intentionally keeps provenance, authorization, retry row,
// outbox, and audit event in one unit; splitting it would weaken the contract.
#[allow(clippy::too_many_lines)]
pub async fn retry_run(
    authorizer: &PostgresMelangeAuthorizer,
    transaction: &mut Transaction<'_, Postgres>,
    command: &ControlCommand,
) -> Result<ControlOutcome, ReviewRepositoryError> {
    let source_run_id = command
        .run_id
        .ok_or(ReviewRepositoryError::DeliveryMismatch)?;
    let source = sqlx::query_as::<_, RetrySource>(
        "SELECT request.repository_id, request.commit_sha, request.git_ref,
                request.receive_id, request.instance_id,
                request.instance_revision_id, request.release_id,
                request.release_agent_id, request.attachment_id,
                request.platform_policy_version, request.requires_state
         FROM run_requests request
         WHERE request.run_id = $1 FOR UPDATE",
    )
    .bind(source_run_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(db_error)?;
    let Some(source) = source else {
        close_rejected(transaction, command.command_id, "retry_unsupported").await?;
        return Ok(ControlOutcome::Rejected);
    };
    if source.repository_id != command.repository_id.as_uuid() {
        return Err(ReviewRepositoryError::DeliveryMismatch);
    }
    let decision = authorize(
        authorizer,
        transaction,
        command,
        Permission::CanExecute,
        ObjectRef::new(ObjectType::AgentInstance, source.instance_id),
    )
    .await?;
    if !decision.is_allowed() {
        close_denied(transaction, command.command_id).await?;
        return Ok(ControlOutcome::Denied);
    }
    let attempt: i32 = sqlx::query_scalar(
        "SELECT COALESCE(max(attempt), 0) + 1
         FROM run_requests
         WHERE repository_id = $1 AND commit_sha = $2 AND git_ref = $3
           AND attachment_id = $4 AND instance_revision_id = $5
           AND receive_id = $6",
    )
    .bind(source.repository_id)
    .bind(&source.commit_sha)
    .bind(&source.git_ref)
    .bind(source.attachment_id)
    .bind(source.instance_revision_id)
    .bind(source.receive_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(db_error)?;
    let run_id = RunId::new();
    let start_id = CommandId::new();
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, receive_id,
          instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, platform_policy_version, request_kind,
          run_id, command_id, actor_id, request_id, retry_of_run_id, attempt,
          requires_state)
         VALUES
         ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
          'instance_normal', $12, $13, $14, $15, $16, $17, $18)",
    )
    .bind(Uuid::new_v4())
    .bind(source.repository_id)
    .bind(&source.commit_sha)
    .bind(&source.git_ref)
    .bind(source.receive_id)
    .bind(source.instance_id)
    .bind(source.instance_revision_id)
    .bind(source.release_id)
    .bind(source.release_agent_id)
    .bind(source.attachment_id)
    .bind(&source.platform_policy_version)
    .bind(run_id.as_uuid())
    .bind(start_id.as_uuid())
    .bind(command.actor_id.as_uuid())
    .bind(command.request_id.as_uuid())
    .bind(source_run_id.as_uuid())
    .bind(attempt)
    .bind(source.requires_state)
    .execute(&mut **transaction)
    .await
    .map_err(db_error)?;
    let start = StartRun {
        command_id: start_id,
        run_id,
        instance_id: AgentInstanceId::from_uuid(source.instance_id),
        instance_revision_id: AgentInstanceRevisionId::from_uuid(source.instance_revision_id),
        release_id: ReleaseId::from_uuid(source.release_id),
        release_agent_id: ReleaseAgentId::from_uuid(source.release_agent_id),
        attachment_id: Some(RuntimeAttachmentId::from_uuid(source.attachment_id)),
        kind: RunKind::Normal,
        requires_state: source.requires_state,
    };
    insert_outbox(
        transaction,
        "run_request",
        run_id.as_uuid(),
        START_RUN_SUBJECT,
        "run.retry_requested",
        serde_json::to_value(start).map_err(|error| infrastructure(error.to_string()))?,
    )
    .await?;
    complete_control(transaction, command.command_id).await?;
    append_run_event(
        transaction,
        source_run_id,
        "review.retry_requested",
        serde_json::json!({
            "actor_id": command.actor_id,
            "retry_run_id": run_id,
            "attempt": attempt,
        }),
    )
    .await?;
    Ok(ControlOutcome::Completed)
}

pub async fn reject_result(
    authorizer: &PostgresMelangeAuthorizer,
    transaction: &mut Transaction<'_, Postgres>,
    command: &ControlCommand,
) -> Result<ControlOutcome, ReviewRepositoryError> {
    let proposal_id = command
        .proposal_id
        .ok_or(ReviewRepositoryError::DeliveryMismatch)?;
    let proposal = ProposalRow::load(transaction, proposal_id).await?;
    proposal.matches(command)?;
    let decision = authorize(
        authorizer,
        transaction,
        command,
        Permission::CanWrite,
        ObjectRef::new(ObjectType::Repository, proposal.repository_id),
    )
    .await?;
    if !decision.is_allowed() {
        close_denied(transaction, command.command_id).await?;
        return Ok(ControlOutcome::Denied);
    }
    if proposal.state == "approved" {
        return Err(ReviewRepositoryError::ProposalClosed(proposal.state));
    }
    sqlx::query(
        "UPDATE review_proposals
         SET state = 'rejected', version = version + 1,
             decision_actor_id = $2, decision_request_id = $3,
             decision_reason = $4, decided_at = now(), updated_at = now()
         WHERE id = $1 AND state IN ('open', 'approval_requested')",
    )
    .bind(proposal.id)
    .bind(command.actor_id.as_uuid())
    .bind(command.request_id.as_uuid())
    .bind(&command.reason)
    .execute(&mut **transaction)
    .await
    .map_err(db_error)?;
    complete_control(transaction, command.command_id).await?;
    append_run_event(
        transaction,
        RunId::from_uuid(proposal.run_id),
        "review.rejected",
        serde_json::json!({"actor_id": command.actor_id, "reason": command.reason}),
    )
    .await?;
    Ok(ControlOutcome::Completed)
}

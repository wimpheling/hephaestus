//! PostgreSQL-backed human controls and trusted result-ref publication.

use async_nats::{HeaderMap, jetstream};
use authz_domain::{AuthorizationDecision, Authorizer, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use forge_domain::{CommitSha, GitRef, RepositoryId, RunRequestId};
use forge_service::GitStorage;
use identity_domain::{RequestId, UserId};
use review_domain::{
    CONTROL_EXECUTE_SUBJECT, ControlCommand, ControlKind, ControlRequestId, ReviewProposalId,
};
use run_domain::{RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sqlx::{PgPool, Postgres, Transaction};
use std::{path::Path, process::Stdio, sync::Arc};
use time::OffsetDateTime;
use tokio::process::Command;
use uuid::Uuid;

const CANCEL_RUN_SUBJECT: &str = "heph.run.command.cancel.v1";
const START_RUN_SUBJECT: &str = "hephaestus.run.start";

/// Publishes browser control intents and their derived retry commands.
#[derive(Clone)]
pub struct ReviewOutboxPublisher {
    context: jetstream::Context,
    pool: PgPool,
}

impl ReviewOutboxPublisher {
    /// Creates a publisher for review-owned outbox aggregates.
    #[must_use]
    pub const fn new(context: jetstream::Context, pool: PgPool) -> Self {
        Self { context, pool }
    }

    /// Publishes committed control and retry records with `JetStream`
    /// deduplication identifiers.
    ///
    /// # Errors
    ///
    /// Returns after persisting the first database or publication failure.
    pub async fn publish_pending(&self, limit: i64) -> Result<usize, ReviewOutboxPublishError> {
        let rows = sqlx::query_as::<_, ReviewOutboxRow>(
            "SELECT id, subject, payload FROM outbox
             WHERE published_at IS NULL
               AND aggregate_type IN ('control_request', 'run_request')
             ORDER BY occurred_at, id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let count = rows.len();
        for row in rows {
            let payload = serde_json::to_vec(&row.payload)?;
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", row.id.to_string());
            let publication = self
                .context
                .publish_with_headers(row.subject, headers, payload.into())
                .await;
            let result = match publication {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_) => {
                    sqlx::query(
                        "UPDATE outbox
                         SET published_at = now(), attempts = attempts + 1,
                             last_error = NULL
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .execute(&self.pool)
                    .await?;
                }
                Err(error) => {
                    sqlx::query(
                        "UPDATE outbox
                         SET attempts = attempts + 1, last_error = $2
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .bind(&error)
                    .execute(&self.pool)
                    .await?;
                    return Err(ReviewOutboxPublishError::JetStream(error));
                }
            }
        }
        Ok(count)
    }
}

#[derive(sqlx::FromRow)]
struct ReviewOutboxRow {
    id: Uuid,
    subject: String,
    payload: serde_json::Value,
}

/// Review outbox database, serialization, or publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReviewOutboxPublishError {
    /// `PostgreSQL` access failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// Command serialization failed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` rejected publication.
    #[error("JetStream publication failed: {0}")]
    JetStream(String),
}

/// Result of idempotently processing one durable human control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOutcome {
    /// The requested operation completed.
    Completed,
    /// A prior delivery already completed the operation.
    AlreadyCompleted,
    /// Authorization denied the operation and the request was closed.
    Denied,
    /// The Git target moved and the proposal was marked conflicted.
    Conflicted,
}

/// Trusted host service for browser-originated run and review commands.
#[derive(Clone)]
pub struct ReviewControlService {
    pool: PgPool,
    storage: Arc<GitStorage>,
    authorizer: PostgresMelangeAuthorizer,
}

impl ReviewControlService {
    /// Creates a service over worker database access and canonical Git storage.
    #[must_use]
    pub const fn new(pool: PgPool, storage: Arc<GitStorage>) -> Self {
        Self {
            pool,
            storage,
            authorizer: PostgresMelangeAuthorizer,
        }
    }

    /// Processes one outbox-derived command with durable idempotency.
    ///
    /// Authorization is re-evaluated by the Rust worker. Run commands are
    /// converted into another transactional outbox record. Result approval is
    /// the sole operation that touches canonical Git storage and uses a
    /// compare-and-swap ref update.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid delivery, database failure, or Git
    /// validation/publication failure. Retrying the same command is safe.
    pub async fn execute(
        &self,
        command: &ControlCommand,
    ) -> Result<ControlOutcome, ControlServiceError> {
        command.validate()?;
        let mut transaction = self.pool.begin().await?;
        set_actor(&mut transaction, command.actor_id, command.request_id).await?;
        let row = ControlRow::load(&mut transaction, command.command_id).await?;
        row.matches(command)?;
        if row.state == "completed" || row.state == "failed" {
            transaction.commit().await?;
            return Ok(ControlOutcome::AlreadyCompleted);
        }

        match command.kind {
            ControlKind::CancelRun => {
                let outcome = self.cancel_run(&mut transaction, command).await?;
                transaction.commit().await?;
                Ok(outcome)
            }
            ControlKind::RetryRun => {
                let outcome = self.retry_run(&mut transaction, command).await?;
                transaction.commit().await?;
                Ok(outcome)
            }
            ControlKind::RejectResult => {
                let outcome = self.reject_result(&mut transaction, command).await?;
                transaction.commit().await?;
                Ok(outcome)
            }
            ControlKind::ApproveResult => {
                let preparation = self.authorize_approval(&mut transaction, command).await?;
                transaction.commit().await?;
                match preparation {
                    ApprovalPreparation::Ready(proposal) => {
                        self.approve_result(command, &proposal).await
                    }
                    ApprovalPreparation::Terminal(outcome) => Ok(outcome),
                }
            }
        }
    }

    async fn cancel_run(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        command: &ControlCommand,
    ) -> Result<ControlOutcome, ControlServiceError> {
        let run_id = command.run_id.ok_or(ControlServiceError::InvalidTarget)?;
        let decision = self
            .authorize(
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
        let cancel_id = CommandId::from_uuid(command.command_id.as_uuid());
        insert_outbox(
            transaction,
            "run",
            run_id.as_uuid(),
            CANCEL_RUN_SUBJECT,
            "run.cancel_requested",
            serde_json::json!({
                "command_id": cancel_id,
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

    // The authorization, exact provenance copy, outbox, and audit transition
    // form one transaction and are clearest when reviewed together.
    #[allow(clippy::too_many_lines)]
    async fn retry_run(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        command: &ControlCommand,
    ) -> Result<ControlOutcome, ControlServiceError> {
        let source_run_id = command.run_id.ok_or(ControlServiceError::InvalidTarget)?;
        let source = sqlx::query_as::<_, RetrySource>(
            "SELECT request.repository_id, request.commit_sha, request.git_ref,
                    request.receive_id, request.instance_id,
                    request.instance_revision_id, request.release_id,
                    request.release_agent_id, request.attachment_id,
                    request.platform_policy_version, request.requires_state,
                    request.attempt
             FROM run_requests request
             WHERE request.run_id = $1
             FOR UPDATE",
        )
        .bind(source_run_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ControlServiceError::MissingRunRequest(source_run_id))?;
        if source.repository_id != command.repository_id.as_uuid() {
            return Err(ControlServiceError::DeliveryMismatch);
        }
        let instance_id = AgentInstanceId::from_uuid(source.instance_id);
        let decision = self
            .authorize(
                transaction,
                command,
                Permission::CanExecute,
                ObjectRef::new(ObjectType::AgentInstance, instance_id.as_uuid()),
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
        .await?;
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
        .bind(RunRequestId::new().as_uuid())
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
        .await?;
        let start = StartRun {
            command_id: start_id,
            run_id,
            instance_id,
            instance_revision_id: AgentInstanceRevisionId::from_uuid(source.instance_revision_id),
            release_id: ReleaseId::from_uuid(source.release_id),
            release_agent_id: ReleaseAgentId::from_uuid(source.release_agent_id),
            attachment_id: Some(AgentAttachmentId::from_uuid(source.attachment_id)),
            kind: RunKind::Normal,
            requires_state: source.requires_state,
        };
        insert_outbox(
            transaction,
            "run_request",
            run_id.as_uuid(),
            START_RUN_SUBJECT,
            "run.retry_requested",
            serde_json::to_value(start)?,
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

    async fn reject_result(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        command: &ControlCommand,
    ) -> Result<ControlOutcome, ControlServiceError> {
        let proposal = ProposalRow::load(
            transaction,
            command
                .proposal_id
                .ok_or(ControlServiceError::InvalidTarget)?,
        )
        .await?;
        proposal.matches(command)?;
        let decision = self
            .authorize(
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
            return Err(ControlServiceError::ProposalClosed(proposal.state));
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
        .await?;
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

    async fn authorize_approval(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        command: &ControlCommand,
    ) -> Result<ApprovalPreparation, ControlServiceError> {
        let proposal = ProposalRow::load(
            transaction,
            command
                .proposal_id
                .ok_or(ControlServiceError::InvalidTarget)?,
        )
        .await?;
        proposal.matches(command)?;
        let decision = self
            .authorize(
                transaction,
                command,
                Permission::CanWrite,
                ObjectRef::new(ObjectType::Repository, proposal.repository_id),
            )
            .await?;
        if !decision.is_allowed() {
            close_denied(transaction, command.command_id).await?;
            return Ok(ApprovalPreparation::Terminal(ControlOutcome::Denied));
        }
        match proposal.state.as_str() {
            "open" | "approval_requested" => {}
            "approved" => {
                complete_control(transaction, command.command_id).await?;
                return Ok(ApprovalPreparation::Terminal(
                    ControlOutcome::AlreadyCompleted,
                ));
            }
            _ => return Err(ControlServiceError::ProposalClosed(proposal.state.clone())),
        }
        sqlx::query("UPDATE control_requests SET state = 'processing' WHERE id = $1")
            .bind(command.command_id.as_uuid())
            .execute(&mut **transaction)
            .await?;
        sqlx::query(
            "UPDATE review_proposals
             SET state = 'approval_requested', version = version + 1,
                 decision_actor_id = $2, decision_request_id = $3,
                 decision_reason = $4, updated_at = now()
             WHERE id = $1 AND state IN ('open', 'approval_requested')",
        )
        .bind(proposal.id)
        .bind(command.actor_id.as_uuid())
        .bind(command.request_id.as_uuid())
        .bind(&command.reason)
        .execute(&mut **transaction)
        .await?;
        Ok(ApprovalPreparation::Ready(proposal))
    }

    async fn approve_result(
        &self,
        command: &ControlCommand,
        proposal: &ProposalRow,
    ) -> Result<ControlOutcome, ControlServiceError> {
        let repository_id = RepositoryId::from_uuid(proposal.repository_id);
        let repository = self.storage.validate_existing(repository_id).await?;
        let target_ref = GitRef::parse(proposal.target_ref.clone())?;
        let result_ref = GitRef::parse(proposal.result_ref.clone())?;
        let input = CommitSha::parse(proposal.input_commit.clone())?;
        let result = CommitSha::parse(proposal.result_commit.clone())?;
        validate_result_provenance(&repository, &result_ref, &result, &input).await?;
        let current = resolve_ref(&repository, &target_ref).await?;
        let outcome = if current.as_ref() == Some(&result) {
            ControlOutcome::Completed
        } else if current.as_ref() != Some(&input) {
            ControlOutcome::Conflicted
        } else {
            cas_update_ref(&repository, &target_ref, &result, &input).await?;
            ControlOutcome::Completed
        };

        let mut transaction = self.pool.begin().await?;
        set_actor(&mut transaction, command.actor_id, command.request_id).await?;
        let locked = ProposalRow::load(
            &mut transaction,
            command
                .proposal_id
                .ok_or(ControlServiceError::InvalidTarget)?,
        )
        .await?;
        let (state, event_type) = match outcome {
            ControlOutcome::Conflicted => ("conflicted", "review.conflicted"),
            _ => ("approved", "review.approved"),
        };
        sqlx::query(
            "UPDATE review_proposals
             SET state = $2, version = version + 1, decided_at = now(),
                 updated_at = now()
             WHERE id = $1 AND state = 'approval_requested'",
        )
        .bind(locked.id)
        .bind(state)
        .execute(&mut *transaction)
        .await?;
        complete_control(&mut transaction, command.command_id).await?;
        append_run_event(
            &mut transaction,
            RunId::from_uuid(locked.run_id),
            event_type,
            serde_json::json!({
                "actor_id": command.actor_id,
                "target_ref": locked.target_ref,
                "input_commit": locked.input_commit,
                "result_commit": locked.result_commit,
            }),
        )
        .await?;
        transaction.commit().await?;
        Ok(outcome)
    }

    async fn authorize(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        command: &ControlCommand,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<AuthorizationDecision, ControlServiceError> {
        let decision = self
            .authorizer
            .check(
                transaction,
                Subject::User(command.actor_id),
                permission,
                object,
            )
            .await?;
        audit_decision(
            transaction,
            command.actor_id,
            permission,
            object,
            decision,
            command.request_id,
        )
        .await?;
        Ok(decision)
    }
}

/// `JetStream` adapter which acknowledges only after durable command effects.
#[derive(Clone)]
pub struct NatsControlHandler {
    service: ReviewControlService,
}

impl NatsControlHandler {
    /// Creates a handler.
    #[must_use]
    pub const fn new(service: ReviewControlService) -> Self {
        Self { service }
    }

    /// Decodes, processes, and confirms one control delivery.
    ///
    /// # Errors
    ///
    /// Returns without acknowledging on a processing or acknowledgement
    /// failure, allowing `JetStream` redelivery.
    pub async fn handle(
        &self,
        message: &async_nats::jetstream::Message,
    ) -> Result<ControlOutcome, ControlHandlingError> {
        if message.message.subject.as_str() != CONTROL_EXECUTE_SUBJECT {
            return Err(ControlHandlingError::UnknownSubject(
                message.message.subject.to_string(),
            ));
        }
        let command: ControlCommand = serde_json::from_slice(&message.payload)?;
        let result = self.service.execute(&command).await?;
        message
            .double_ack()
            .await
            .map_err(|error| ControlHandlingError::Acknowledgement(error.to_string()))?;
        Ok(result)
    }
}

#[derive(sqlx::FromRow)]
struct ControlRow {
    id: Uuid,
    kind: String,
    actor_id: Uuid,
    request_id: Uuid,
    repository_id: Uuid,
    run_id: Option<Uuid>,
    proposal_id: Option<Uuid>,
    reason: String,
    state: String,
}

impl ControlRow {
    async fn load(
        transaction: &mut Transaction<'_, Postgres>,
        id: ControlRequestId,
    ) -> Result<Self, ControlServiceError> {
        sqlx::query_as(
            "SELECT id, kind, actor_id, request_id, repository_id,
                    run_id, proposal_id, reason, state
             FROM control_requests WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ControlServiceError::MissingControl(id))
    }

    fn matches(&self, command: &ControlCommand) -> Result<(), ControlServiceError> {
        if self.id != command.command_id.as_uuid()
            || self.kind != kind_name(command.kind)
            || self.actor_id != command.actor_id.as_uuid()
            || self.request_id != command.request_id.as_uuid()
            || self.repository_id != command.repository_id.as_uuid()
            || self.run_id != command.run_id.map(RunId::as_uuid)
            || self.proposal_id != command.proposal_id.map(ReviewProposalId::as_uuid)
            || self.reason != command.reason
        {
            return Err(ControlServiceError::DeliveryMismatch);
        }
        Ok(())
    }
}

enum ApprovalPreparation {
    Ready(ProposalRow),
    Terminal(ControlOutcome),
}

#[derive(sqlx::FromRow)]
struct ProposalRow {
    id: Uuid,
    repository_id: Uuid,
    run_id: Uuid,
    target_ref: String,
    input_commit: String,
    result_commit: String,
    result_ref: String,
    state: String,
}

impl ProposalRow {
    async fn load(
        transaction: &mut Transaction<'_, Postgres>,
        id: ReviewProposalId,
    ) -> Result<Self, ControlServiceError> {
        sqlx::query_as(
            "SELECT id, repository_id, run_id, target_ref, input_commit,
                    result_commit, result_ref, state
             FROM review_proposals WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ControlServiceError::MissingProposal(id))
    }

    fn matches(&self, command: &ControlCommand) -> Result<(), ControlServiceError> {
        if self.repository_id != command.repository_id.as_uuid() {
            return Err(ControlServiceError::DeliveryMismatch);
        }
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct RetrySource {
    repository_id: Uuid,
    commit_sha: String,
    git_ref: String,
    receive_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Uuid,
    platform_policy_version: String,
    requires_state: bool,
    #[allow(dead_code)]
    attempt: i32,
}

async fn set_actor(
    transaction: &mut Transaction<'_, Postgres>,
    actor_id: UserId,
    request_id: RequestId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true)",
    )
    .bind(actor_id.to_string())
    .bind(request_id.to_string())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn close_denied(
    transaction: &mut Transaction<'_, Postgres>,
    id: ControlRequestId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE control_requests
         SET state = 'failed',
             diagnostics = jsonb_build_array(
                 jsonb_build_object('code', 'authorization_denied')
             ),
             processed_at = now()
         WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn complete_control(
    transaction: &mut Transaction<'_, Postgres>,
    id: ControlRequestId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE control_requests
         SET state = 'completed', processed_at = now()
         WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn insert_outbox(
    transaction: &mut Transaction<'_, Postgres>,
    aggregate_type: &str,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    payload: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type,
          payload, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(subject)
    .bind(event_type)
    .bind(payload)
    .bind(OffsetDateTime::now_utc())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn append_run_event(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    event_type: &str,
    payload: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run_id.as_uuid())
        .execute(&mut **transaction)
        .await?;
    sqlx::query(
        "INSERT INTO run_events
         (id, run_id, sequence, event_type, payload, occurred_at)
         SELECT $1, $2, COALESCE(max(sequence), 0) + 1, $3, $4, $5
         FROM run_events WHERE run_id = $2",
    )
    .bind(Uuid::new_v4())
    .bind(run_id.as_uuid())
    .bind(event_type)
    .bind(payload)
    .bind(OffsetDateTime::now_utc())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn validate_result_provenance(
    repository: &Path,
    result_ref: &GitRef,
    result: &CommitSha,
    input: &CommitSha,
) -> Result<(), ControlServiceError> {
    let published = resolve_ref(repository, result_ref).await?;
    if published.as_ref() != Some(result) {
        return Err(ControlServiceError::InvalidResultProvenance(String::from(
            "controlled result ref does not point at the recorded result",
        )));
    }
    let parent = git_text(repository, &["rev-parse", &format!("{}^", result.as_str())]).await?;
    if parent != input.as_str() {
        return Err(ControlServiceError::InvalidResultProvenance(String::from(
            "result commit parent is not the exact input commit",
        )));
    }
    Ok(())
}

async fn resolve_ref(
    repository: &Path,
    git_ref: &GitRef,
) -> Result<Option<CommitSha>, ControlServiceError> {
    let output = git_output(repository, &["rev-parse", "--verify", git_ref.as_str()]).await?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Ok(Some(CommitSha::parse(value)?));
    }
    Ok(None)
}

async fn cas_update_ref(
    repository: &Path,
    target: &GitRef,
    result: &CommitSha,
    input: &CommitSha,
) -> Result<(), ControlServiceError> {
    let output = git_output(
        repository,
        &[
            "update-ref",
            target.as_str(),
            result.as_str(),
            input.as_str(),
        ],
    )
    .await?;
    if !output.status.success() {
        return Err(ControlServiceError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(())
}

async fn git_text(repository: &Path, arguments: &[&str]) -> Result<String, ControlServiceError> {
    let output = git_output(repository, arguments).await?;
    if !output.status.success() {
        return Err(ControlServiceError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn git_output(
    repository: &Path,
    arguments: &[&str],
) -> Result<std::process::Output, ControlServiceError> {
    Command::new("git")
        .arg("--git-dir")
        .arg(repository)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(ControlServiceError::Io)
}

const fn kind_name(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::CancelRun => "cancel_run",
        ControlKind::RetryRun => "retry_run",
        ControlKind::ApproveResult => "approve_result",
        ControlKind::RejectResult => "reject_result",
    }
}

/// Durable control processing failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ControlServiceError {
    /// Command targets did not match its operation.
    #[error(transparent)]
    InvalidCommand(#[from] review_domain::InvalidControlCommand),
    /// Command omitted its required target.
    #[error("control command target is missing")]
    InvalidTarget,
    /// A delivery did not match its authoritative database row.
    #[error("control delivery does not match its authoritative database row")]
    DeliveryMismatch,
    /// The durable control request does not exist.
    #[error("control request {0} does not exist")]
    MissingControl(ControlRequestId),
    /// The review proposal does not exist.
    #[error("review proposal {0} does not exist")]
    MissingProposal(ReviewProposalId),
    /// The source run did not originate from an accepted forge request.
    #[error("run {0} has no forge run request")]
    MissingRunRequest(RunId),
    /// Authorization denied the operation.
    #[error("authorization denied the control operation")]
    AuthorizationDenied,
    /// The proposal was already approved.
    #[error("review proposal is already approved")]
    ProposalAlreadyApproved,
    /// The proposal is no longer actionable.
    #[error("review proposal is closed in state {0}")]
    ProposalClosed(String),
    /// Recorded result provenance failed trusted host validation.
    #[error("invalid result provenance: {0}")]
    InvalidResultProvenance(String),
    /// A Git value stored in the database was invalid.
    #[error(transparent)]
    GitValue(#[from] forge_domain::GitValueError),
    /// Canonical repository resolution failed.
    #[error(transparent)]
    Storage(#[from] forge_service::GitStorageError),
    /// Database processing failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// Authorization evaluation failed.
    #[error(transparent)]
    Authorization(#[from] authz_domain::AuthzError),
    /// Exact start-command serialization failed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// Git process launch failed.
    #[error("Git process failed: {0}")]
    Io(#[source] std::io::Error),
    /// Git rejected an operation.
    #[error("Git operation failed: {0}")]
    Git(String),
}

/// Control delivery failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ControlHandlingError {
    /// Delivery used an unsupported subject.
    #[error("unsupported control subject {0}")]
    UnknownSubject(String),
    /// Delivery payload was not a valid command.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// Durable processing failed.
    #[error(transparent)]
    Service(#[from] ControlServiceError),
    /// `JetStream` did not confirm acknowledgement.
    #[error("control acknowledgement failed: {0}")]
    Acknowledgement(String),
}

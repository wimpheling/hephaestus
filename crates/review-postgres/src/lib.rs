//! `PostgreSQL` persistence adapters for review controls and command outbox.

use async_trait::async_trait;
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use forge_domain::RepositoryId;
use forge_service::GitStorage;
use release_domain::{AgentInstanceRevisionId, ReleaseAgentId, ReleaseId};
use review_domain::{ControlCommand, ControlKind, ControlRequestId, ReviewProposalId};
use review_service::{
    ApprovalDisposition, ApprovalPreparation, ApprovalProposal, ControlOutcome, RepositoryLocator,
    ReviewOutboxRecord, ReviewOutboxStore, ReviewOutboxStoreError, ReviewRepository,
    ReviewRepositoryError,
};
use run_domain::{RunKind, StartRun};
use runtime_types::{AgentAttachmentId as RuntimeAttachmentId, AgentInstanceId, CommandId, RunId};
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

const CANCEL_RUN_SUBJECT: &str = "heph.run.command.cancel.v1";
const START_RUN_SUBJECT: &str = "hephaestus.run.start";

/// PostgreSQL-backed review repository implementing the provider-neutral service port.
#[derive(Clone)]
pub struct PostgresReviewRepository {
    pool: PgPool,
    authorizer: PostgresMelangeAuthorizer,
}

impl PostgresReviewRepository {
    /// Creates an adapter over the supplied `PostgreSQL` pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            authorizer: PostgresMelangeAuthorizer,
        }
    }

    /// Returns the underlying pool for adapter composition and health checks.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// Canonical Git repository locator backed by forge storage.
#[derive(Clone)]
pub struct GitRepositoryLocator {
    storage: Arc<GitStorage>,
}

impl GitRepositoryLocator {
    /// Creates a locator over canonical bare-Git storage.
    #[must_use]
    pub const fn new(storage: Arc<GitStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl RepositoryLocator for GitRepositoryLocator {
    async fn locate(&self, repository_id: RepositoryId) -> Result<std::path::PathBuf, String> {
        self.storage
            .validate_existing(repository_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl ReviewRepository for PostgresReviewRepository {
    async fn execute_control(
        &self,
        command: &ControlCommand,
    ) -> Result<ControlOutcome, ReviewRepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(db_error)?;
        set_actor(&mut transaction, command)
            .await
            .map_err(db_error)?;
        let row = ControlRow::load(&mut transaction, command.command_id).await?;
        row.matches(command)?;
        if row.is_terminal() {
            transaction.commit().await.map_err(db_error)?;
            return Ok(ControlOutcome::AlreadyCompleted);
        }
        let outcome = match command.kind {
            ControlKind::CancelRun => {
                cancel_run(&self.authorizer, &mut transaction, command).await?
            }
            ControlKind::RetryRun => retry_run(&self.authorizer, &mut transaction, command).await?,
            ControlKind::RejectResult => {
                reject_result(&self.authorizer, &mut transaction, command).await?
            }
            ControlKind::ApproveResult => {
                return Err(ReviewRepositoryError::Infrastructure(
                    "approval must use prepare_approval".to_owned(),
                ));
            }
        };
        transaction.commit().await.map_err(db_error)?;
        Ok(outcome)
    }

    async fn prepare_approval(
        &self,
        command: &ControlCommand,
    ) -> Result<ApprovalPreparation, ReviewRepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(db_error)?;
        set_actor(&mut transaction, command)
            .await
            .map_err(db_error)?;
        let control = ControlRow::load(&mut transaction, command.command_id).await?;
        control.matches(command)?;
        if control.is_terminal() {
            transaction.commit().await.map_err(db_error)?;
            return Ok(ApprovalPreparation::Terminal(
                ControlOutcome::AlreadyCompleted,
            ));
        }
        let proposal_id = command
            .proposal_id
            .ok_or(ReviewRepositoryError::DeliveryMismatch)?;
        let proposal = ProposalRow::load(&mut transaction, proposal_id).await?;
        proposal.matches(command)?;
        let decision = authorize(
            &self.authorizer,
            &mut transaction,
            command,
            Permission::CanWrite,
            ObjectRef::new(ObjectType::Repository, proposal.repository_id),
        )
        .await?;
        if !decision.is_allowed() {
            close_denied(&mut transaction, command.command_id).await?;
            transaction.commit().await.map_err(db_error)?;
            return Ok(ApprovalPreparation::Terminal(ControlOutcome::Denied));
        }
        match proposal.state.as_str() {
            "open" | "approval_requested" => {}
            "approved" => {
                complete_control(&mut transaction, command.command_id).await?;
                transaction.commit().await.map_err(db_error)?;
                return Ok(ApprovalPreparation::Terminal(
                    ControlOutcome::AlreadyCompleted,
                ));
            }
            state => return Err(ReviewRepositoryError::ProposalClosed(state.to_owned())),
        }
        sqlx::query("UPDATE control_requests SET state = 'processing' WHERE id = $1")
            .bind(command.command_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(db_error)?;
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
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
        transaction.commit().await.map_err(db_error)?;
        Ok(ApprovalPreparation::Ready(proposal.into_service()))
    }

    async fn finalize_approval(
        &self,
        command: &ControlCommand,
        proposal: &ApprovalProposal,
        disposition: ApprovalDisposition,
    ) -> Result<ControlOutcome, ReviewRepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(db_error)?;
        set_actor(&mut transaction, command)
            .await
            .map_err(db_error)?;
        let control = ControlRow::load(&mut transaction, command.command_id).await?;
        control.matches(command)?;
        if control.is_terminal() {
            transaction.commit().await.map_err(db_error)?;
            return Ok(ControlOutcome::AlreadyCompleted);
        }
        let locked = ProposalRow::load(&mut transaction, proposal.id).await?;
        locked.matches(command)?;
        if locked.state == "approved" || locked.state == "conflicted" {
            transaction.commit().await.map_err(db_error)?;
            return Ok(ControlOutcome::AlreadyCompleted);
        }
        if locked.state != "approval_requested"
            || locked.input_commit != proposal.input_commit
            || locked.result_commit != proposal.result_commit
        {
            return Err(ReviewRepositoryError::DeliveryMismatch);
        }
        let (state, event_type, outcome) = match disposition {
            ApprovalDisposition::Approved => {
                ("approved", "review.approved", ControlOutcome::Completed)
            }
            ApprovalDisposition::Conflicted => (
                "conflicted",
                "review.conflicted",
                ControlOutcome::Conflicted,
            ),
        };
        sqlx::query(
            "UPDATE review_proposals
             SET state = $2, version = version + 1, decided_at = now(), updated_at = now()
             WHERE id = $1 AND state = 'approval_requested'",
        )
        .bind(locked.id)
        .bind(state)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
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
        transaction.commit().await.map_err(db_error)?;
        Ok(outcome)
    }
}

#[async_trait]
impl ReviewOutboxStore for PostgresReviewRepository {
    async fn claim_pending(
        &self,
        subjects: &[&str],
        limit: i64,
    ) -> Result<Vec<ReviewOutboxRecord>, ReviewOutboxStoreError> {
        let subjects: Vec<String> = subjects
            .iter()
            .map(|subject| (*subject).to_owned())
            .collect();
        sqlx::query_as::<_, OutboxRow>(
            "SELECT id, subject, payload FROM outbox
             WHERE published_at IS NULL AND subject = ANY($1)
             ORDER BY occurred_at, id LIMIT $2",
        )
        .bind(subjects)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(store_error)
    }

    async fn mark_published(&self, id: Uuid) -> Result<(), ReviewOutboxStoreError> {
        sqlx::query(
            "UPDATE outbox SET published_at = now(), attempts = attempts + 1,
                    last_error = NULL WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(store_error)
    }

    async fn mark_failed(&self, id: Uuid, error: &str) -> Result<(), ReviewOutboxStoreError> {
        sqlx::query("UPDATE outbox SET attempts = attempts + 1, last_error = $2 WHERE id = $1")
            .bind(id)
            .bind(error)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(store_error)
    }
}

#[derive(Debug, FromRow)]
struct OutboxRow {
    id: Uuid,
    subject: String,
    payload: Value,
}

impl From<OutboxRow> for ReviewOutboxRecord {
    fn from(row: OutboxRow) -> Self {
        Self {
            id: row.id,
            subject: row.subject,
            payload: row.payload,
        }
    }
}

#[derive(Debug, FromRow)]
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
    ) -> Result<Self, ReviewRepositoryError> {
        sqlx::query_as(
            "SELECT id, kind, actor_id, request_id, repository_id,
                    run_id, proposal_id, reason, state
             FROM control_requests WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(db_error)?
        .ok_or(ReviewRepositoryError::MissingControl(id))
    }

    fn matches(&self, command: &ControlCommand) -> Result<(), ReviewRepositoryError> {
        if self.id != command.command_id.as_uuid()
            || self.kind != kind_name(command.kind)
            || self.actor_id != command.actor_id.as_uuid()
            || self.request_id != command.request_id.as_uuid()
            || self.repository_id != command.repository_id.as_uuid()
            || self.run_id != command.run_id.map(RunId::as_uuid)
            || self.proposal_id != command.proposal_id.map(ReviewProposalId::as_uuid)
            || self.reason != command.reason
        {
            return Err(ReviewRepositoryError::DeliveryMismatch);
        }
        Ok(())
    }

    fn is_terminal(&self) -> bool {
        self.state == "completed" || self.state == "failed"
    }
}

#[derive(Debug, FromRow)]
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
    ) -> Result<Self, ReviewRepositoryError> {
        sqlx::query_as(
            "SELECT id, repository_id, run_id, target_ref, input_commit,
                    result_commit, result_ref, state
             FROM review_proposals WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(db_error)?
        .ok_or(ReviewRepositoryError::MissingProposal(id))
    }

    fn matches(&self, command: &ControlCommand) -> Result<(), ReviewRepositoryError> {
        if self.repository_id != command.repository_id.as_uuid() {
            return Err(ReviewRepositoryError::DeliveryMismatch);
        }
        Ok(())
    }

    fn into_service(self) -> ApprovalProposal {
        ApprovalProposal {
            id: ReviewProposalId::from_uuid(self.id),
            repository_id: RepositoryId::from_uuid(self.repository_id),
            run_id: RunId::from_uuid(self.run_id),
            target_ref: self.target_ref,
            input_commit: self.input_commit,
            result_commit: self.result_commit,
            result_ref: self.result_ref,
        }
    }
}

#[derive(Debug, FromRow)]
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
}

async fn cancel_run(
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
async fn retry_run(
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
    .map_err(db_error)?
    .ok_or(ReviewRepositoryError::MissingRunRequest(source_run_id))?;
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

async fn reject_result(
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

async fn authorize(
    authorizer: &PostgresMelangeAuthorizer,
    transaction: &mut Transaction<'_, Postgres>,
    command: &ControlCommand,
    permission: Permission,
    object: ObjectRef,
) -> Result<authz_domain::AuthorizationDecision, ReviewRepositoryError> {
    let decision = authorizer
        .check(
            transaction,
            Subject::User(command.actor_id),
            permission,
            object,
        )
        .await
        .map_err(|error| infrastructure(error.to_string()))?;
    audit_decision(
        transaction,
        command.actor_id,
        permission,
        object,
        decision,
        command.request_id,
    )
    .await
    .map_err(db_error)?;
    Ok(decision)
}

async fn set_actor(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ControlCommand,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true)",
    )
    .bind(command.actor_id.to_string())
    .bind(command.request_id.to_string())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
}

async fn close_denied(
    transaction: &mut Transaction<'_, Postgres>,
    id: ControlRequestId,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query(
        "UPDATE control_requests
         SET state = 'failed', diagnostics = jsonb_build_array(
             jsonb_build_object('code', 'authorization_denied')),
             processed_at = now() WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(db_error)
}

async fn complete_control(
    transaction: &mut Transaction<'_, Postgres>,
    id: ControlRequestId,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query(
        "UPDATE control_requests SET state = 'completed', processed_at = now() WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(db_error)
}

async fn insert_outbox(
    transaction: &mut Transaction<'_, Postgres>,
    aggregate_type: &str,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    payload: Value,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
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
    .await
    .map(|_| ())
    .map_err(db_error)
}

async fn append_run_event(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    event_type: &str,
    payload: Value,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(db_error)?;
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
    .await
    .map(|_| ())
    .map_err(db_error)
}

const fn kind_name(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::CancelRun => "cancel_run",
        ControlKind::RetryRun => "retry_run",
        ControlKind::ApproveResult => "approve_result",
        ControlKind::RejectResult => "reject_result",
    }
}

// The error is owned because `sqlx::Error` is returned by `map_err`; converting
// it immediately preserves its provider detail in the port's string error.
#[allow(clippy::needless_pass_by_value)]
fn db_error(error: sqlx::Error) -> ReviewRepositoryError {
    infrastructure(error.to_string())
}

const fn infrastructure(error: String) -> ReviewRepositoryError {
    ReviewRepositoryError::Infrastructure(error)
}

// See `db_error`: ownership comes from `map_err` and is consumed into text.
#[allow(clippy::needless_pass_by_value)]
fn store_error(error: sqlx::Error) -> ReviewOutboxStoreError {
    ReviewOutboxStoreError(error.to_string())
}

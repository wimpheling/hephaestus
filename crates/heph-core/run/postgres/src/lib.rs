//! `PostgreSQL` persistence adapter for durable runs.

mod runtime_catalog;

use async_trait::async_trait;
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use run_domain::{CancelRun, InvalidTransition, Run, RunKind, RunOutcome, RunState, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository, StoredVmEvent};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, EventId, LeaseId, ReleaseAgentId,
    ReleaseId, RunId, VolumeId,
};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Row, Transaction, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmExit;

/// `PostgreSQL` implementation of [`RunRepository`].
#[derive(Clone)]
pub struct PgRunRepository {
    pub(crate) pool: PgPool,
}

impl PgRunRepository {
    /// Creates a repository using an existing connection pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn locked_run(
        transaction: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<RunRow, RepositoryError> {
        sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id = $1 FOR UPDATE")
            .bind(run_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run_id))
    }

    async fn append_event_tx(
        transaction: &mut Transaction<'_, Postgres>,
        run_id: RunId,
        event_type: &str,
        payload: Value,
        occurred_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM run_events WHERE run_id = $1",
        )
        .bind(run_id.as_uuid())
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
        let event_id = EventId::new();
        sqlx::query(
            "INSERT INTO run_events
             (id, run_id, sequence, event_type, payload, occurred_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(event_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(sequence)
        .bind(event_type)
        .bind(&payload)
        .bind(occurred_at)
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
        Ok(())
    }
}

#[derive(Debug, sqlx::FromRow)]
struct RuntimeGitProvenance {
    publication_mode: String,
    trigger_repository_id: Option<Uuid>,
    git_ref: Option<String>,
    commit_sha: Option<String>,
    attachment_id: Option<Uuid>,
    git_binding_id: Option<Uuid>,
    target_repository_id: Option<Uuid>,
    parameter_hash: Vec<u8>,
    platform_policy_version: String,
}

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct ProvenanceRow {
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Option<Uuid>,
    target_repository_id: Option<Uuid>,
    target_ref: Option<String>,
    target_commit: Option<String>,
    parameter_hash: Vec<u8>,
    platform_policy_version: String,
    phase: String,
    authorization_model_version: String,
}

#[async_trait]
impl RunRepository for PgRunRepository {
    async fn create_run(&self, command: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let payload = serde_json::to_value(command).map_err(storage)?;
        let inbox = sqlx::query(
            "INSERT INTO command_inbox (command_id, command_type, payload)
             VALUES ($1, 'start_run', $2)
             ON CONFLICT (command_id) DO NOTHING",
        )
        .bind(command.command_id.as_uuid())
        .bind(payload)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        if inbox.rows_affected() == 0 {
            let row = sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE command_id = $1")
                .bind(command.command_id.as_uuid())
                .fetch_one(&mut *transaction)
                .await
                .map_err(storage)?;
            transaction.commit().await.map_err(storage)?;
            return Ok(CreateRunResult {
                run: row.try_into()?,
                created: false,
            });
        }
        if let Some(row) = sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id = $1")
            .bind(command.run_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(storage)?
        {
            let run: Run = row.try_into()?;
            if !matches_start_command(&run, command) || run.state != RunState::Queued {
                return Err(RepositoryError::InvalidData(
                    "precreated update run does not match start command",
                ));
            }
            let now = OffsetDateTime::now_utc();
            sqlx::query("UPDATE command_inbox SET processed_at = $2 WHERE command_id = $1")
                .bind(command.command_id.as_uuid())
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
            transaction.commit().await.map_err(storage)?;
            return Ok(CreateRunResult { run, created: true });
        }

        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "INSERT INTO runs
             (id, instance_id, instance_revision_id, release_id,
              release_agent_id, attachment_id, run_kind, command_id, state,
              requires_state, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'queued', $9, $10, $10)",
        )
        .bind(command.run_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid())
        .bind(command.release_id.as_uuid())
        .bind(command.release_agent_id.as_uuid())
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(run_kind_name(command.kind))
        .bind(command.command_id.as_uuid())
        .bind(command.requires_state)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        sqlx::query(
            "UPDATE run_requests
             SET dispatch_state = 'dispatched'
             WHERE run_id = $1 AND command_id = $2
               AND dispatch_state = 'pending'",
        )
        .bind(command.run_id.as_uuid())
        .bind(command.command_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        Self::append_event_tx(
            &mut transaction,
            command.run_id,
            "run.queued",
            json!({"run_id": command.run_id, "state": "queued"}),
            now,
        )
        .await?;
        sqlx::query("UPDATE command_inbox SET processed_at = $2 WHERE command_id = $1")
            .bind(command.command_id.as_uuid())
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(CreateRunResult {
            run: self.get(command.run_id).await?,
            created: true,
        })
    }

    // Keep the derivation, insert, and immutable replay comparison together so
    // the security-sensitive target capture remains auditable in one boundary.
    #[allow(clippy::too_many_lines)]
    async fn ensure_runtime_git_provenance(&self, run: &Run) -> Result<(), RepositoryError> {
        if run.kind != RunKind::Normal {
            return Ok(());
        }
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let expected: RuntimeGitProvenance = sqlx::query_as(
            "SELECT revision.publication_mode,
                    request.repository_id AS trigger_repository_id,
                    request.git_ref,
                    request.commit_sha,
                    request.attachment_id,
                    git.binding_id AS git_binding_id,
                    generic.resource_id AS target_repository_id,
                    revision.parameter_hash,
                    revision.platform_policy_version
             FROM runs AS stored_run
             JOIN agent_instance_revisions AS revision
               ON revision.id = stored_run.instance_revision_id
              AND revision.instance_id = stored_run.instance_id
             LEFT JOIN run_requests AS request
               ON request.run_id = stored_run.id
              AND request.instance_id = stored_run.instance_id
              AND request.instance_revision_id = stored_run.instance_revision_id
              AND request.release_id = stored_run.release_id
              AND request.release_agent_id = stored_run.release_agent_id
             LEFT JOIN agent_capability_bindings AS generic
               ON generic.id = revision.publication_repository_binding_id
              AND generic.instance_revision_id = revision.id
              AND generic.resource_kind = 'repository'
             LEFT JOIN agent_git_capability_bindings AS git
               ON git.binding_id = generic.id
              AND git.instance_revision_id = revision.id
             WHERE stored_run.id = $1
               AND stored_run.instance_id = $2
               AND stored_run.instance_revision_id = $3
               AND stored_run.release_id = $4
               AND stored_run.release_agent_id = $5
             FOR UPDATE OF stored_run",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(RepositoryError::InvalidData("runtime Git run"))?;

        if expected.publication_mode != "runtime_git" {
            transaction.commit().await.map_err(storage)?;
            return Ok(());
        }
        let attachment_id = run.attachment_id.map(AgentAttachmentId::as_uuid).ok_or(
            RepositoryError::InvalidData("runtime Git normal run attachment"),
        )?;
        let target_repository_id =
            expected
                .target_repository_id
                .ok_or(RepositoryError::InvalidData(
                    "runtime Git publication repository capability",
                ))?;
        if expected.git_binding_id.is_none()
            || expected.trigger_repository_id.is_none()
            || expected.attachment_id != Some(attachment_id)
            || expected.git_ref.is_none()
            || expected.commit_sha.is_none()
        {
            return Err(RepositoryError::InvalidData(
                "runtime Git immutable run request",
            ));
        }

        sqlx::query(
            "INSERT INTO run_instance_provenance
               (run_id, instance_id, instance_revision_id, release_id,
                release_agent_id, attachment_id, target_repository_id,
                target_ref, target_commit, parameter_hash,
                platform_policy_version, phase, authorization_model_version)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     $11, 'normal', $12)
             ON CONFLICT (run_id) DO NOTHING",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(attachment_id)
        .bind(target_repository_id)
        .bind(expected.git_ref.as_deref())
        .bind(expected.commit_sha.as_deref())
        .bind(&expected.parameter_hash)
        .bind(&expected.platform_policy_version)
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;

        let actual: ProvenanceRow = sqlx::query_as(
            "SELECT instance_id, instance_revision_id, release_id,
                    release_agent_id, attachment_id, target_repository_id,
                    target_ref, target_commit, parameter_hash,
                    platform_policy_version, phase, authorization_model_version
             FROM run_instance_provenance
             WHERE run_id = $1
             FOR UPDATE",
        )
        .bind(run.id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(RepositoryError::InvalidData(
            "runtime Git provenance disappeared",
        ))?;
        let expected_provenance = ProvenanceRow {
            instance_id: run.instance_id.as_uuid(),
            instance_revision_id: run.instance_revision_id.as_uuid(),
            release_id: run.release_id.as_uuid(),
            release_agent_id: run.release_agent_id.as_uuid(),
            attachment_id: Some(attachment_id),
            target_repository_id: Some(target_repository_id),
            target_ref: Some(
                expected
                    .git_ref
                    .as_deref()
                    .ok_or(RepositoryError::InvalidData("runtime Git ref"))?
                    .to_owned(),
            ),
            target_commit: Some(
                expected
                    .commit_sha
                    .as_deref()
                    .ok_or(RepositoryError::InvalidData("runtime Git commit"))?
                    .to_owned(),
            ),
            parameter_hash: expected.parameter_hash.clone(),
            platform_policy_version: expected.platform_policy_version.clone(),
            phase: String::from("normal"),
            authorization_model_version: String::from(AUTHORIZATION_MODEL_VERSION),
        };
        if actual != expected_provenance {
            return Err(RepositoryError::InvalidData(
                "runtime Git provenance conflict",
            ));
        }
        transaction.commit().await.map_err(storage)?;
        Ok(())
    }

    async fn get(&self, run_id: RunId) -> Result<Run, RepositoryError> {
        sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id = $1")
            .bind(run_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run_id))?
            .try_into()
    }

    async fn bind_resources(
        &self,
        run_id: RunId,
        volume_id: Option<VolumeId>,
        lease_id: Option<LeaseId>,
        lease_fencing_token: Option<i64>,
        vm_id: &str,
    ) -> Result<Run, RepositoryError> {
        sqlx::query(
            "UPDATE runs
             SET volume_id = $2, lease_id = $3, lease_fencing_token = $4,
                 vm_id = $5, updated_at = now()
             WHERE id = $1",
        )
        .bind(run_id.as_uuid())
        .bind(volume_id.map(VolumeId::as_uuid))
        .bind(lease_id.map(LeaseId::as_uuid))
        .bind(lease_fencing_token)
        .bind(vm_id)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        self.get(run_id).await
    }

    async fn transition(
        &self,
        run_id: RunId,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let current = Self::locked_run(&mut transaction, run_id).await?;
        let current_state = parse_state(&current.state)?;
        if !current_state.can_transition_to(next) {
            return Err(InvalidTransition {
                current: current_state,
                requested: next,
            }
            .into());
        }
        let outcome = next.outcome().map(outcome_name);
        let exit_code = exit.and_then(|value| value.code);
        let exit_signal = exit.and_then(|value| value.signal);
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "UPDATE runs
             SET state = $2,
                 outcome = COALESCE($3, outcome),
                 exit_code = COALESCE($4, exit_code),
                 exit_signal = COALESCE($5, exit_signal),
                 failure = COALESCE($6, failure),
                 updated_at = $7,
                 state_version = state_version + 1
             WHERE id = $1",
        )
        .bind(run_id.as_uuid())
        .bind(state_name(next))
        .bind(outcome)
        .bind(exit_code)
        .bind(exit_signal)
        .bind(failure)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        Self::append_event_tx(
            &mut transaction,
            run_id,
            &format!("run.{}", state_name(next)),
            json!({
                "run_id": run_id,
                "state": state_name(next),
                "outcome": outcome,
                "exit": exit.map(|value| json!({
                    "code": value.code,
                    "signal": value.signal
                })),
                "failure": failure
            }),
            now,
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        self.get(run_id).await
    }

    async fn append_vm_event(
        &self,
        run_id: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let _run = Self::locked_run(&mut transaction, run_id).await?;
        Self::append_event_tx(
            &mut transaction,
            run_id,
            &event.event_type,
            event.payload,
            event.occurred_at,
        )
        .await?;
        transaction.commit().await.map_err(storage)
    }

    async fn request_cancel(&self, command: &CancelRun) -> Result<bool, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let inserted = sqlx::query(
            "INSERT INTO command_inbox (command_id, command_type, payload)
             VALUES ($1, 'cancel_run', $2)
             ON CONFLICT (command_id) DO NOTHING",
        )
        .bind(command.command_id.as_uuid())
        .bind(serde_json::to_value(command).map_err(storage)?)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?
        .rows_affected()
            == 1;
        if inserted {
            let now = OffsetDateTime::now_utc();
            let updated = sqlx::query(
                "UPDATE runs SET cancel_requested_at = COALESCE(cancel_requested_at, $2),
                 updated_at = $2 WHERE id = $1",
            )
            .bind(command.run_id.as_uuid())
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?
            .rows_affected();
            if updated == 0 {
                return Err(RepositoryError::NotFound(command.run_id));
            }
            sqlx::query("UPDATE command_inbox SET processed_at = $2 WHERE command_id = $1")
                .bind(command.command_id.as_uuid())
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
        }
        transaction.commit().await.map_err(storage)?;
        Ok(inserted)
    }

    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        sqlx::query_as::<_, RunRow>(
            "SELECT * FROM runs WHERE state <> 'cleaned_up' ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }
}

fn matches_start_command(run: &Run, command: &StartRun) -> bool {
    [
        run.id == command.run_id,
        run.command_id == command.command_id,
        run.instance_id == command.instance_id,
        run.instance_revision_id == command.instance_revision_id,
        run.release_id == command.release_id,
        run.release_agent_id == command.release_agent_id,
        run.attachment_id == command.attachment_id,
        run.kind == command.kind,
        run.requires_state == command.requires_state,
    ]
    .into_iter()
    .all(std::convert::identity)
}

#[derive(Debug)]
struct RunRow {
    id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Option<Uuid>,
    run_kind: String,
    requires_state: bool,
    command_id: Uuid,
    volume_id: Option<Uuid>,
    lease_id: Option<Uuid>,
    lease_fencing_token: Option<i64>,
    vm_id: Option<String>,
    state: String,
    outcome: Option<String>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    failure: Option<String>,
    cancel_requested_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    state_version: i64,
}

impl<'row> FromRow<'row, PgRow> for RunRow {
    fn from_row(row: &'row PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            instance_id: row.try_get("instance_id")?,
            instance_revision_id: row.try_get("instance_revision_id")?,
            release_id: row.try_get("release_id")?,
            release_agent_id: row.try_get("release_agent_id")?,
            attachment_id: row.try_get("attachment_id")?,
            run_kind: row.try_get("run_kind")?,
            requires_state: row.try_get("requires_state")?,
            command_id: row.try_get("command_id")?,
            volume_id: row.try_get("volume_id")?,
            lease_id: row.try_get("lease_id")?,
            lease_fencing_token: row.try_get("lease_fencing_token")?,
            vm_id: row.try_get("vm_id")?,
            state: row.try_get("state")?,
            outcome: row.try_get("outcome")?,
            exit_code: row.try_get("exit_code")?,
            exit_signal: row.try_get("exit_signal")?,
            failure: row.try_get("failure")?,
            cancel_requested_at: row.try_get("cancel_requested_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            state_version: row.try_get("state_version")?,
        })
    }
}

impl TryFrom<RunRow> for Run {
    type Error = RepositoryError;

    fn try_from(row: RunRow) -> Result<Self, Self::Error> {
        let exit = if row.exit_code.is_some() || row.exit_signal.is_some() {
            Some(VmExit {
                code: row.exit_code,
                signal: row.exit_signal,
            })
        } else {
            None
        };
        Ok(Self {
            id: RunId::from_uuid(row.id),
            instance_id: AgentInstanceId::from_uuid(row.instance_id),
            instance_revision_id: AgentInstanceRevisionId::from_uuid(row.instance_revision_id),
            release_id: ReleaseId::from_uuid(row.release_id),
            release_agent_id: ReleaseAgentId::from_uuid(row.release_agent_id),
            attachment_id: row.attachment_id.map(AgentAttachmentId::from_uuid),
            kind: parse_run_kind(&row.run_kind)?,
            requires_state: row.requires_state,
            command_id: row.command_id.into(),
            volume_id: row.volume_id.map(Into::into),
            lease_id: row.lease_id.map(Into::into),
            lease_fencing_token: row.lease_fencing_token,
            vm_id: row.vm_id,
            state: parse_state(&row.state)?,
            outcome: row.outcome.as_deref().map(parse_outcome).transpose()?,
            exit,
            failure: row.failure,
            cancel_requested_at: row.cancel_requested_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            state_version: row.state_version,
        })
    }
}

const fn run_kind_name(kind: RunKind) -> &'static str {
    match kind {
        RunKind::Normal => "normal",
        RunKind::Update => "update",
    }
}

fn parse_run_kind(value: &str) -> Result<RunKind, RepositoryError> {
    match value {
        "normal" => Ok(RunKind::Normal),
        "update" => Ok(RunKind::Update),
        _ => Err(RepositoryError::InvalidData("run kind")),
    }
}

const fn state_name(state: RunState) -> &'static str {
    match state {
        RunState::Queued => "queued",
        RunState::LeasingVolume => "leasing_volume",
        RunState::Provisioning => "provisioning",
        RunState::Starting => "starting",
        RunState::Running => "running",
        RunState::Succeeded => "succeeded",
        RunState::Failed => "failed",
        RunState::Cancelled => "cancelled",
        RunState::CleaningUp => "cleaning_up",
        RunState::CleanedUp => "cleaned_up",
        _ => "unsupported",
    }
}

fn parse_state(value: &str) -> Result<RunState, RepositoryError> {
    match value {
        "queued" => Ok(RunState::Queued),
        "leasing_volume" => Ok(RunState::LeasingVolume),
        "provisioning" => Ok(RunState::Provisioning),
        "starting" => Ok(RunState::Starting),
        "running" => Ok(RunState::Running),
        "succeeded" => Ok(RunState::Succeeded),
        "failed" => Ok(RunState::Failed),
        "cancelled" => Ok(RunState::Cancelled),
        "cleaning_up" => Ok(RunState::CleaningUp),
        "cleaned_up" => Ok(RunState::CleanedUp),
        _ => Err(RepositoryError::InvalidData("unknown run state")),
    }
}

const fn outcome_name(outcome: RunOutcome) -> &'static str {
    match outcome {
        RunOutcome::Succeeded => "succeeded",
        RunOutcome::Failed => "failed",
        RunOutcome::Cancelled => "cancelled",
        _ => "unsupported",
    }
}

fn parse_outcome(value: &str) -> Result<RunOutcome, RepositoryError> {
    match value {
        "succeeded" => Ok(RunOutcome::Succeeded),
        "failed" => Ok(RunOutcome::Failed),
        "cancelled" => Ok(RunOutcome::Cancelled),
        _ => Err(RepositoryError::InvalidData("unknown run outcome")),
    }
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> RepositoryError {
    RepositoryError::Storage(Box::new(error))
}

use run_domain::{Run, RunKind, RunOutcome, RunState};
use run_orchestrator::RepositoryError;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId, RunId,
};
use sqlx::{FromRow, Row, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmExit;

pub fn matches_start_command(run: &Run, command: &run_domain::StartRun) -> bool {
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
pub struct RunRow {
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
    pub(super) state: String,
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

pub const fn run_kind_name(kind: RunKind) -> &'static str {
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

pub const fn state_name(state: RunState) -> &'static str {
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

pub fn parse_state(value: &str) -> Result<RunState, RepositoryError> {
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

pub const fn outcome_name(outcome: RunOutcome) -> &'static str {
    match outcome {
        RunOutcome::Succeeded => "succeeded",
        RunOutcome::Failed => "failed",
        RunOutcome::Cancelled => "cancelled",
        _ => "unsupported",
    }
}

pub fn parse_outcome(value: &str) -> Result<RunOutcome, RepositoryError> {
    match value {
        "succeeded" => Ok(RunOutcome::Succeeded),
        "failed" => Ok(RunOutcome::Failed),
        "cancelled" => Ok(RunOutcome::Cancelled),
        _ => Err(RepositoryError::InvalidData("unknown run outcome")),
    }
}

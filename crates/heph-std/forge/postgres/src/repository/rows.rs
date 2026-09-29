use forge_domain::{
    CommitSha, GitRef, OrganizationId, Project, ProjectId, ReceiveId, Repository, RepositoryId,
    RunRequestId,
};
use forge_service::{ForgeRepositoryError, RunRequest};
use run_domain::{RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use serde_json::Value;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(FromRow)]
pub struct ProjectRow {
    pub id: Uuid,
    organization_id: Uuid,
    name: String,
    created_at: OffsetDateTime,
}

impl From<ProjectRow> for Project {
    fn from(row: ProjectRow) -> Self {
        Self {
            id: ProjectId::from_uuid(row.id),
            organization_id: OrganizationId::from_uuid(row.organization_id),
            name: row.name,
            created_at: row.created_at,
        }
    }
}

#[derive(FromRow)]
pub struct RepositoryRow {
    id: Uuid,
    project_id: Uuid,
    name: String,
    default_branch: String,
    is_public: bool,
    settings: Value,
    created_at: OffsetDateTime,
}

#[derive(FromRow)]
pub struct ExistingReceiveProvenance {
    pub repository: Uuid,
    pub runtime_session: Option<Uuid>,
    pub runtime_attachment: Option<Uuid>,
}

impl TryFrom<RepositoryRow> for Repository {
    type Error = ForgeRepositoryError;

    fn try_from(row: RepositoryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: RepositoryId::from_uuid(row.id),
            project_id: ProjectId::from_uuid(row.project_id),
            name: row.name,
            default_branch: GitRef::parse(row.default_branch)
                .map_err(|_| ForgeRepositoryError::InvalidStoredData("default_branch"))?,
            is_public: row.is_public,
            agent_runs_enabled: row
                .settings
                .get("agent_runs_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            created_at: row.created_at,
        })
    }
}

#[derive(FromRow)]
pub struct RunRequestRow {
    id: Uuid,
    repository_id: Uuid,
    commit_sha: String,
    git_ref: String,
    receive_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Uuid,
    run_id: Uuid,
    command_id: Uuid,
    requires_state: bool,
}

impl TryFrom<RunRequestRow> for RunRequest {
    type Error = ForgeRepositoryError;

    fn try_from(row: RunRequestRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: RunRequestId::from_uuid(row.id),
            repository_id: RepositoryId::from_uuid(row.repository_id),
            commit_sha: CommitSha::parse(row.commit_sha)
                .map_err(|_| ForgeRepositoryError::InvalidStoredData("commit_sha"))?,
            git_ref: GitRef::parse(row.git_ref)
                .map_err(|_| ForgeRepositoryError::InvalidStoredData("git_ref"))?,
            receive_id: ReceiveId::from_uuid(row.receive_id),
            command: StartRun {
                command_id: CommandId::from_uuid(row.command_id),
                run_id: RunId::from_uuid(row.run_id),
                instance_id: AgentInstanceId::from_uuid(row.instance_id),
                instance_revision_id: AgentInstanceRevisionId::from_uuid(row.instance_revision_id),
                release_id: ReleaseId::from_uuid(row.release_id),
                release_agent_id: ReleaseAgentId::from_uuid(row.release_agent_id),
                attachment_id: Some(AgentAttachmentId::from_uuid(row.attachment_id)),
                kind: RunKind::Normal,
                requires_state: row.requires_state,
            },
        })
    }
}

#[derive(FromRow)]
pub struct OutboxRow {
    pub id: Uuid,
    pub subject: String,
    pub payload: Value,
}

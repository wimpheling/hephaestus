use crate::common::{
    AgentConfig, AgentInstanceId, DEFAULT_RESULT_MESSAGE, LocalWorkspaceError, PublicationMode,
    PublishedResult, RepositoryId, ResultId, RunRequest, SOURCE_GUEST_PATH, serialization,
};
use run_domain::RunKind;
use workspace_domain::{ResultMetadata, WorkspaceRequestMetadata};

impl TryFrom<WorkspaceRequestMetadata> for RunRequest {
    type Error = LocalWorkspaceError;

    fn try_from(row: WorkspaceRequestMetadata) -> Result<Self, Self::Error> {
        Ok(Self {
            repository_id: RepositoryId::from_uuid(row.repository_id),
            commit: row.commit_sha,
            instance_id: AgentInstanceId::from_uuid(row.instance_id),
            config: serde_json::from_value(row.configuration).map_err(serialization)?,
        })
    }
}

pub fn published_result(row: ResultMetadata) -> Result<PublishedResult, LocalWorkspaceError> {
    Ok(PublishedResult {
        id: ResultId::from_uuid(row.id),
        result_ref: row.result_ref,
        result_commit: row.result_commit.ok_or_else(|| {
            LocalWorkspaceError::State(String::from("completed result has no commit"))
        })?,
        result_tree: row.result_tree.ok_or_else(|| {
            LocalWorkspaceError::State(String::from("completed result has no tree"))
        })?,
    })
}

pub fn parse_run_kind(value: &str) -> Result<RunKind, LocalWorkspaceError> {
    match value {
        "normal" => Ok(RunKind::Normal),
        "update" => Ok(RunKind::Update),
        _ => Err(LocalWorkspaceError::State(String::from(
            "stored run kind is invalid",
        ))),
    }
}

pub fn validate_mount_policy(config: &AgentConfig) -> Result<(), LocalWorkspaceError> {
    if config.publication.mode != PublicationMode::Proposal
        || config.publication.mode.permits_git_write_remote()
    {
        return Err(LocalWorkspaceError::Configuration(String::from(
            "controlled workspaces are available only to proposal-mode releases",
        )));
    }
    if config.workspace.path != SOURCE_GUEST_PATH || !config.workspace.read_only {
        return Err(LocalWorkspaceError::Configuration(format!(
            "workspace source mount must request read-only {SOURCE_GUEST_PATH}"
        )));
    }
    Ok(())
}

pub fn validate_message(message: &str) -> Result<&str, LocalWorkspaceError> {
    let message = if message.trim().is_empty() {
        DEFAULT_RESULT_MESSAGE
    } else {
        message
    };
    if message.len() > 4096 || message.contains('\0') {
        return Err(LocalWorkspaceError::InvalidResult(String::from(
            "result message must contain at most 4096 UTF-8 bytes and no NUL",
        )));
    }
    Ok(message)
}

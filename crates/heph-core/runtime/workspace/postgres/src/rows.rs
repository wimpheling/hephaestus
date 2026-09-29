//! SQL row mappings and transactional event insertion.

use runtime_types::RunId;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;
use workspace_domain::{
    PendingResultMetadata, ResultMetadata, RuntimeGitWorkspaceRequest, WorkspaceMetadata,
    WorkspaceRepositoryError, WorkspaceRequestMetadata,
};

use super::error::error;

pub async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: RunId,
    event_type: &str,
    payload: Value,
) -> Result<(), WorkspaceRepositoryError> {
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run_id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(error)?;
    let sequence: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sequence),0)+1 FROM run_events WHERE run_id=$1")
            .bind(run_id.as_uuid())
            .fetch_one(&mut **tx)
            .await
            .map_err(error)?;
    sqlx::query("INSERT INTO run_events (id,run_id,sequence,event_type,payload,occurred_at) VALUES ($1,$2,$3,$4,$5,now())").bind(Uuid::new_v4()).bind(run_id.as_uuid()).bind(sequence).bind(event_type).bind(payload).execute(&mut **tx).await.map_err(error)?;
    Ok(())
}

#[derive(FromRow)]
pub struct RequestRow {
    pub(super) repository_id: Uuid,
    pub(super) commit_sha: String,
    pub(super) instance_id: Uuid,
    pub(super) configuration: Value,
}

#[derive(FromRow)]
pub struct RuntimeGitRequestRow {
    pub(super) repository_id: Uuid,
    pub(super) target_repository_id: Uuid,
    pub(super) target_ref: String,
    pub(super) target_commit: String,
    pub(super) git_operations: Vec<String>,
    pub(super) ref_globs: Vec<String>,
}

#[derive(FromRow)]
pub struct RuntimeGitWorkspaceRow {
    pub(super) run_id: Uuid,
    pub(super) id: Uuid,
    pub(super) state: String,
    pub(super) active_path: String,
    pub(super) sealed_path: String,
    pub(super) input_commit: Option<String>,
}

impl From<RuntimeGitWorkspaceRow> for WorkspaceMetadata {
    fn from(row: RuntimeGitWorkspaceRow) -> Self {
        Self {
            id: row.id,
            state: row.state,
            active_path: row.active_path,
            sealed_path: row.sealed_path,
            input_commit: row.input_commit,
        }
    }
}

impl From<RuntimeGitRequestRow> for RuntimeGitWorkspaceRequest {
    fn from(row: RuntimeGitRequestRow) -> Self {
        Self {
            repository_id: row.repository_id,
            target_repository_id: row.target_repository_id,
            target_ref: row.target_ref,
            target_commit: row.target_commit,
            git_operations: row.git_operations,
            ref_globs: row.ref_globs,
        }
    }
}

impl From<RequestRow> for WorkspaceRequestMetadata {
    fn from(row: RequestRow) -> Self {
        Self {
            repository_id: row.repository_id,
            commit_sha: row.commit_sha,
            instance_id: row.instance_id,
            configuration: row.configuration,
        }
    }
}
#[derive(FromRow)]
pub struct WorkspaceRow {
    pub(super) id: Uuid,
    pub(super) state: String,
    pub(super) active_path: String,
    pub(super) sealed_path: String,
    pub(super) input_commit: Option<String>,
}
impl From<WorkspaceRow> for WorkspaceMetadata {
    fn from(row: WorkspaceRow) -> Self {
        Self {
            id: row.id,
            state: row.state,
            active_path: row.active_path,
            sealed_path: row.sealed_path,
            input_commit: row.input_commit,
        }
    }
}
#[derive(FromRow)]
pub struct ResultRow {
    pub(super) id: Uuid,
    pub(super) run_id: Uuid,
    pub(super) repository_id: Uuid,
    pub(super) result_ref: String,
    pub(super) result_commit: Option<String>,
    pub(super) result_tree: Option<String>,
}
impl From<ResultRow> for ResultMetadata {
    fn from(row: ResultRow) -> Self {
        Self {
            id: row.id,
            run_id: row.run_id,
            repository_id: row.repository_id,
            result_ref: row.result_ref,
            result_commit: row.result_commit,
            result_tree: row.result_tree,
        }
    }
}
#[derive(FromRow)]
pub struct PendingRow {
    pub(super) run_id: Uuid,
    pub(super) message: String,
    pub(super) command_id: Uuid,
    pub(super) instance_id: Uuid,
    pub(super) instance_revision_id: Uuid,
    pub(super) release_id: Uuid,
    pub(super) release_agent_id: Uuid,
    pub(super) attachment_id: Option<Uuid>,
    pub(super) run_kind: String,
    pub(super) requires_state: bool,
    pub(super) created_at: time::OffsetDateTime,
}
impl From<PendingRow> for PendingResultMetadata {
    fn from(row: PendingRow) -> Self {
        Self {
            run_id: row.run_id,
            message: row.message,
            command_id: row.command_id,
            instance_id: row.instance_id,
            instance_revision_id: row.instance_revision_id,
            release_id: row.release_id,
            release_agent_id: row.release_agent_id,
            attachment_id: row.attachment_id,
            run_kind: row.run_kind,
            requires_state: row.requires_state,
            created_at: row.created_at,
        }
    }
}

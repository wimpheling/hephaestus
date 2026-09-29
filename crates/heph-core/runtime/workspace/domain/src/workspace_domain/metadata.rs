use async_trait::async_trait;
use runtime_types::RunId;
use serde_json::Value;
use uuid::Uuid;

use super::lifecycle::ResultId;
use super::requests::{
    RuntimeGitWorkspaceRequest, WorkspaceRepositoryError, WorkspaceRequestMetadata,
};

/// Durable workspace row.
#[derive(Debug, Clone)]
pub struct WorkspaceMetadata {
    /// Workspace identifier.
    pub id: Uuid,
    /// State-machine state.
    pub state: String,
    /// Canonical active path.
    pub active_path: String,
    /// Canonical sealed path.
    pub sealed_path: String,
    /// Input commit (present for finalization).
    pub input_commit: Option<String>,
}

/// Durable result row used by recovery and publication.
#[derive(Debug, Clone)]
pub struct ResultMetadata {
    /// Result identifier.
    pub id: Uuid,
    /// Run identifier.
    pub run_id: Uuid,
    /// Repository containing the result ref.
    pub repository_id: Uuid,
    /// Controlled result ref.
    pub result_ref: String,
    /// Imported result commit, when prepared.
    pub result_commit: Option<String>,
    /// Imported result tree, when completed.
    pub result_tree: Option<String>,
}

/// One artifact persisted beside a prepared result.
#[derive(Debug, Clone)]
pub struct ResultArtifactMetadata {
    /// Stable artifact identifier.
    pub id: Uuid,
    /// Artifact kind.
    pub kind: String,
    /// Relative artifact path.
    pub path: String,
    /// Git mode, when applicable.
    pub git_mode: Option<i32>,
    /// MIME type.
    pub media_type: String,
    /// Byte length.
    pub size_bytes: i64,
    /// Content hash.
    pub sha256: String,
    /// Content-addressed storage key.
    pub storage_key: String,
}

/// PostgreSQL-independent metadata port for workspace state and events.
#[async_trait]
pub trait WorkspaceMetadataRepository: Send + Sync + 'static {
    /// Finds the dispatch request for a run command.
    async fn request(
        &self,
        command_id: Uuid,
    ) -> Result<Option<WorkspaceRequestMetadata>, WorkspaceRepositoryError>;
    /// Reads one workspace row.
    async fn workspace(
        &self,
        run_id: RunId,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceRepositoryError>;
    /// Inserts a preparing workspace row.
    async fn insert_preparing(
        &self,
        metadata: &WorkspaceMetadata,
        repository_id: Uuid,
        input_commit: &str,
        run_id: RunId,
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Inserts a runtime-Git workspace and its classification event atomically.
    async fn insert_runtime_git_preparing(
        &self,
        metadata: &WorkspaceMetadata,
        repository_id: Uuid,
        input_commit: &str,
        run_id: RunId,
        event: Value,
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Records materialization failure.
    async fn mark_materialization_failed(
        &self,
        run_id: RunId,
        message: &str,
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Marks a workspace active and records its event atomically.
    async fn mark_active(
        &self,
        run_id: RunId,
        input_tree: &str,
        manifest_hash: &str,
        event: Value,
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Records a lifecycle event transactionally.
    async fn event(
        &self,
        run_id: RunId,
        event_type: &str,
        payload: Value,
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Updates workspace state.
    async fn set_state(&self, run_id: RunId, state: &str) -> Result<(), WorkspaceRepositoryError>;
    /// Marks a workspace failure and state.
    async fn mark_failed(
        &self,
        run_id: RunId,
        state: &str,
        message: &str,
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Marks cleanup complete.
    async fn mark_cleaned(&self, run_id: RunId) -> Result<(), WorkspaceRepositoryError>;

    /// Resolves runtime-Git authority from immutable dispatch rows.
    async fn runtime_git_request(
        &self,
        run_id: RunId,
    ) -> Result<Option<RuntimeGitWorkspaceRequest>, WorkspaceRepositoryError> {
        let _ = run_id;
        Ok(None)
    }

    /// Reads one workspace only when its durable runtime-Git classification is present.
    async fn runtime_git_workspace(
        &self,
        run_id: RunId,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceRepositoryError> {
        let _ = run_id;
        Ok(None)
    }

    /// Lists incomplete runtime-Git workspaces for restart recovery.
    async fn runtime_git_workspaces(
        &self,
    ) -> Result<Vec<(RunId, WorkspaceMetadata)>, WorkspaceRepositoryError> {
        Ok(Vec::new())
    }
}

/// PostgreSQL-independent result persistence and recovery port.
#[async_trait]
#[allow(clippy::too_many_arguments)] // Result insertion mirrors the durable run provenance tuple.
pub trait ResultRepository: Send + Sync + 'static {
    /// Reads durable VM log and exit event payloads for artifact generation.
    async fn vm_logs(
        &self,
        run_id: RunId,
    ) -> Result<(Vec<Value>, Option<Value>), WorkspaceRepositoryError>;
    /// Finds an already completed result.
    async fn completed(
        &self,
        run_id: RunId,
    ) -> Result<Option<ResultMetadata>, WorkspaceRepositoryError>;
    /// Creates the pending result row if absent.
    async fn insert_pending(
        &self,
        run_id: RunId,
        repository_id: Uuid,
        instance_id: Uuid,
        instance_revision_id: Uuid,
        release_id: Uuid,
        release_agent_id: Uuid,
        input_commit: &str,
        result_ref: &str,
        message: &str,
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Marks a result rejected with diagnostics.
    async fn reject(&self, run_id: RunId, message: &str) -> Result<(), WorkspaceRepositoryError>;
    /// Gets the durable result id for a run.
    async fn id_for_run(&self, run_id: RunId) -> Result<ResultId, WorkspaceRepositoryError>;
    /// Persists imported result metadata and artifact rows in one transaction.
    async fn persist_prepared(
        &self,
        result_id: ResultId,
        run_id: RunId,
        tree: &str,
        commit: &str,
        manifest_hash: &str,
        artifacts: &[ResultArtifactMetadata],
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Marks the imported ref as published.
    async fn mark_ref_published(
        &self,
        result_id: ResultId,
        commit: &str,
    ) -> Result<(), WorkspaceRepositoryError>;
    /// Marks a result completed.
    async fn mark_completed(&self, result_id: ResultId) -> Result<(), WorkspaceRepositoryError>;
    /// Lists pending result rows for restart recovery.
    async fn pending(&self) -> Result<Vec<PendingResultMetadata>, WorkspaceRepositoryError>;
    /// Lists prepared/ref-published rows for restart recovery.
    async fn prepared(&self) -> Result<Vec<ResultMetadata>, WorkspaceRepositoryError>;
}

/// Minimal run data needed to reconstruct a pending finalization.
#[derive(Debug, Clone)]
pub struct PendingResultMetadata {
    /// Result run id.
    pub run_id: Uuid,
    /// Requested commit message.
    pub message: String,
    /// Original run fields.
    pub command_id: Uuid,
    /// Agent instance.
    pub instance_id: Uuid,
    /// Agent revision.
    pub instance_revision_id: Uuid,
    /// Release id.
    pub release_id: Uuid,
    /// Release agent id.
    pub release_agent_id: Uuid,
    /// Optional attachment.
    pub attachment_id: Option<Uuid>,
    /// Run kind.
    pub run_kind: String,
    /// State requirement.
    pub requires_state: bool,
    /// Creation timestamp.
    pub created_at: time::OffsetDateTime,
}

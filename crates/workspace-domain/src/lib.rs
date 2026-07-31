//! Provider-neutral repository workspace and agent-result contracts.

use async_trait::async_trait;
use run_domain::Run;
use runtime_types::RunId;
use serde_json::Value;
use std::{error::Error, fmt};
use uuid::Uuid;
use vm_trait::VmMount;

/// Provider-neutral persistence failure for workspace metadata and results.
#[derive(Debug, thiserror::Error)]
#[error("workspace persistence failed: {message}")]
pub struct WorkspaceRepositoryError {
    message: String,
}

impl WorkspaceRepositoryError {
    /// Creates a persistence error from a stable diagnostic.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Metadata needed to prepare one repository workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceRequestMetadata {
    /// Repository containing the requested input commit.
    pub repository_id: Uuid,
    /// Exact input commit to materialize.
    pub commit_sha: String,
    /// Agent instance that requested the run.
    pub instance_id: Uuid,
    /// Serialized agent configuration.
    pub configuration: Value,
}

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

macro_rules! id_type {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new opaque identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Constructs an identifier from its durable UUID.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the durable UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

id_type!(WorkspaceId, "Opaque identifier for one run workspace.");
id_type!(
    ResultId,
    "Opaque identifier for one finalized agent result."
);
id_type!(
    ArtifactId,
    "Opaque identifier for one durable result artifact."
);

/// A workspace prepared for VM attachment.
#[derive(Debug, Clone)]
pub struct PreparedWorkspace {
    /// Durable workspace identifier, absent when workspace mounting is disabled.
    pub id: Option<WorkspaceId>,
    /// Provider-neutral mounts to append to the VM specification.
    pub mounts: Vec<VmMount>,
}

impl PreparedWorkspace {
    /// Returns a disabled workspace with no guest mounts.
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            id: None,
            mounts: Vec::new(),
        }
    }
}

/// One controlled Git result published by the trusted host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedResult {
    /// Durable result identifier.
    pub id: ResultId,
    /// Fully qualified controlled result ref.
    pub result_ref: String,
    /// Host-created result commit object ID.
    pub result_commit: String,
    /// Host-created result tree object ID.
    pub result_tree: String,
}

/// Provider-neutral workspace lifecycle failure.
#[derive(Debug, thiserror::Error)]
#[error("workspace lifecycle operation failed: {0}")]
pub struct WorkspaceError(#[source] Box<dyn Error + Send + Sync>);

impl WorkspaceError {
    /// Wraps an implementation-specific failure.
    #[must_use]
    pub fn operation(error: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

/// Trusted host boundary for run workspace and result lifecycle operations.
#[async_trait]
pub trait RunWorkspaceManager: Send + Sync + 'static {
    /// Materializes the exact input commit and returns approved guest mounts.
    async fn prepare(&self, run: &Run) -> Result<PreparedWorkspace, WorkspaceError>;

    /// Seals, imports, and publishes one finalized result.
    async fn finalize(
        &self,
        run: &Run,
        message: &str,
    ) -> Result<Option<PublishedResult>, WorkspaceError>;

    /// Removes an unfinalized active workspace without publishing a result.
    async fn abandon(&self, run_id: RunId) -> Result<(), WorkspaceError>;

    /// Reconciles incomplete seals and Git ref publications after restart.
    async fn recover(&self) -> Result<usize, WorkspaceError>;
}

/// Workspace manager used when no repository workspace is configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledWorkspaceManager;

#[async_trait]
impl RunWorkspaceManager for DisabledWorkspaceManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedWorkspace, WorkspaceError> {
        Ok(PreparedWorkspace::disabled())
    }

    async fn finalize(
        &self,
        _run: &Run,
        _message: &str,
    ) -> Result<Option<PublishedResult>, WorkspaceError> {
        Ok(None)
    }

    async fn abandon(&self, _run_id: RunId) -> Result<(), WorkspaceError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, WorkspaceError> {
        Ok(0)
    }
}

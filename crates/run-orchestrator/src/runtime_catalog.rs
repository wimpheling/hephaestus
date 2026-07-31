use async_trait::async_trait;
use run_domain::Run;
use runtime_types::RunId;
use serde_json::Value;
use uuid::Uuid;

/// Exact persisted context required to materialize one run runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRuntimeInput {
    /// Current revision parameters written into the guest control directory.
    pub parameters: Value,
    /// Repository identity for a normal run.
    pub repository_id: Option<Uuid>,
    /// Exact repository ref for a normal run.
    pub git_ref: Option<String>,
    /// Exact repository commit for a normal run.
    pub commit_sha: Option<String>,
    /// Durable update identity for an update-hook run.
    pub update_id: Option<Uuid>,
    /// Revision active before an update-hook run.
    pub previous_revision_id: Option<Uuid>,
    /// Release active before an update-hook run.
    pub previous_release_id: Option<Uuid>,
    /// Parameters active before an update-hook run.
    pub previous_parameters: Option<Value>,
    /// Verified metadata for the exact current release.
    pub artifacts: Vec<RunRuntimeArtifact>,
    /// Verified metadata for the previous release during an update.
    pub previous_artifacts: Vec<RunRuntimeArtifact>,
}

/// Bounded release artifact metadata used by local materialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRuntimeArtifact {
    /// Safe relative path declared by the release.
    pub path: String,
    /// Closed artifact kind.
    pub kind: RunRuntimeArtifactKind,
    /// Declared Unix permission bits.
    pub mode: u32,
    /// Exact SHA-256 digest of the canonical object.
    pub content_hash: [u8; 32],
    /// Exact object length.
    pub size_bytes: u64,
    /// Opaque canonical storage identity.
    pub storage_key: Uuid,
}

/// Closed release artifact kinds accepted by the run runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunRuntimeArtifactKind {
    /// Guest-executable regular file.
    Executable,
    /// Non-executable regular file.
    File,
    /// Non-executable manifest file.
    Manifest,
}

/// Provider-neutral runtime catalog failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunRuntimeCatalogError {
    /// Exact runtime provenance is unavailable.
    #[error("exact run runtime provenance is unavailable")]
    Unavailable,
    /// Stored metadata violates the runtime contract.
    #[error("invalid stored runtime metadata: {0}")]
    InvalidData(&'static str),
    /// The persistence provider failed.
    #[error("run runtime catalog operation failed: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence boundary for exact runtime provenance and liveness.
#[async_trait]
pub trait RunRuntimeCatalog: Send + Sync + 'static {
    /// Loads the exact release, revision, target, and artifact metadata for a run.
    async fn load_runtime(&self, run: &Run) -> Result<RunRuntimeInput, RunRuntimeCatalogError>;

    /// Reports whether a run still owns an active local runtime tree.
    async fn run_is_live(&self, run_id: RunId) -> Result<bool, RunRuntimeCatalogError>;
}

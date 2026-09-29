use serde_json::Value;
use uuid::Uuid;

/// Guest location reserved for one isolated runtime-Git worktree.
pub const RUNTIME_GIT_GUEST_PATH: &str = "/workspace/git";
/// Fixed guest loopback port used by the runtime-Git bridge.
pub const RUNTIME_GIT_LOOPBACK_PORT: u16 = 19_100;

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

/// Immutable runtime-Git checkout authority resolved for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeGitWorkspaceRequest {
    /// Repository selected by the immutable runtime capability snapshot.
    pub repository_id: Uuid,
    /// Repository recorded by immutable trigger provenance.
    pub target_repository_id: Uuid,
    /// Fully qualified branch ref selected by immutable run provenance.
    pub target_ref: String,
    /// Exact commit selected by immutable run provenance.
    pub target_commit: String,
    /// Runtime-Git operations copied into the immutable capability snapshot.
    pub git_operations: Vec<String>,
    /// Ref globs copied into the immutable capability snapshot.
    pub ref_globs: Vec<String>,
}

impl RuntimeGitWorkspaceRequest {
    /// Validates the minimum authority needed to materialize a writable
    /// checkout without inferring repository or ref data from the user.
    ///
    /// # Errors
    ///
    /// Returns an error when fetch authority, a scoped branch, or a valid
    /// immutable commit is absent.
    pub fn validate(&self) -> Result<(), WorkspaceRepositoryError> {
        if self.target_repository_id.is_nil() || self.target_repository_id != self.repository_id {
            return Err(WorkspaceRepositoryError::new(
                "runtime Git target repository is not proven by the capability snapshot",
            ));
        }
        if !self
            .git_operations
            .iter()
            .any(|operation| operation == "fetch")
        {
            return Err(WorkspaceRepositoryError::new(
                "runtime Git capability does not authorize fetch",
            ));
        }
        if !self.target_ref.starts_with("refs/heads/")
            || self.target_ref.len() <= "refs/heads/".len()
        {
            return Err(WorkspaceRepositoryError::new(
                "runtime Git target must be a branch ref",
            ));
        }
        if !self
            .ref_globs
            .iter()
            .filter_map(|glob| git_capability_domain::RefGlob::parse_explicitly_broad(glob).ok())
            .any(|glob| glob.is_match(&self.target_ref))
        {
            return Err(WorkspaceRepositoryError::new(
                "runtime Git target ref is outside the immutable capability scope",
            ));
        }
        if !is_commit_sha(&self.target_commit) {
            return Err(WorkspaceRepositoryError::new(
                "runtime Git target commit is invalid",
            ));
        }
        Ok(())
    }
}

fn is_commit_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

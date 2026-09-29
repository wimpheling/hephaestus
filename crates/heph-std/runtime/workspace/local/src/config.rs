pub const SOURCE_GUEST_PATH: &str = "/workspace/repo";
pub const WORK_GUEST_PATH: &str = "/workspace/work";
pub const DEFAULT_RESULT_MESSAGE: &str = "Hephaestus agent result";

/// Resource limits applied while materializing and importing workspaces.
#[derive(Debug, Clone)]
pub struct WorkspaceLimits {
    /// Maximum number of filesystem entries in one tree.
    pub max_entries: usize,
    /// Maximum bytes in one regular file or symlink target.
    pub max_file_bytes: u64,
    /// Maximum aggregate bytes imported from a workspace.
    pub max_total_bytes: u64,
    /// Maximum bytes stored for a generated patch.
    pub max_patch_bytes: usize,
}

impl Default for WorkspaceLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_file_bytes: 32 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_patch_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Configuration for [`LocalWorkspaceManager`](crate::LocalWorkspaceManager).
#[derive(Debug, Clone)]
pub struct LocalWorkspaceConfig {
    /// Root containing active and sealed per-run workspaces.
    pub workspace_root: std::path::PathBuf,
    /// Root containing durable content-addressed result artifacts.
    pub artifact_root: std::path::PathBuf,
    /// Canonical root containing bare repositories.
    pub repository_root: std::path::PathBuf,
    /// Absolute trusted Git executable used only for plumbing commands.
    pub git_binary: std::path::PathBuf,
    /// Materialization and import quotas.
    pub limits: WorkspaceLimits,
}

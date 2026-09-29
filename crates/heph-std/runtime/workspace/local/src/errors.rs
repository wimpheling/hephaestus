use std::io;
use workspace_domain::WorkspaceRepositoryError;

#[allow(clippy::needless_pass_by_value)] // map_err supplies ownership at the persistence boundary.
pub fn repository(error: WorkspaceRepositoryError) -> LocalWorkspaceError {
    LocalWorkspaceError::Persistence(error.to_string())
}

pub const fn serialization(error: serde_json::Error) -> LocalWorkspaceError {
    LocalWorkspaceError::Serialization(error)
}

pub const fn io_error(error: io::Error) -> LocalWorkspaceError {
    LocalWorkspaceError::Io(error)
}

pub const fn join_error(error: tokio::task::JoinError) -> LocalWorkspaceError {
    LocalWorkspaceError::Task(error)
}

pub const fn integer_error(error: std::num::TryFromIntError) -> LocalWorkspaceError {
    LocalWorkspaceError::Integer(error)
}

/// Local workspace and result publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LocalWorkspaceError {
    /// Static configuration is unsafe or incomplete.
    #[error("invalid workspace configuration: {0}")]
    Configuration(String),
    /// Stored or requested lifecycle state is inconsistent.
    #[error("invalid workspace state: {0}")]
    State(String),
    /// A path failed canonical layout validation.
    #[error("unsafe workspace path: {0}")]
    UnsafePath(String),
    /// A repository source tree cannot be safely materialized.
    #[error("invalid repository source: {0}")]
    InvalidSource(String),
    /// A sealed result tree cannot be safely imported.
    #[error("invalid agent result: {0}")]
    InvalidResult(String),
    /// Workspace resource limits were exceeded.
    #[error("workspace quota exceeded: {0}")]
    Quota(String),
    /// A durable or Git publication invariant was violated.
    #[error("workspace integrity failure: {0}")]
    Integrity(String),
    /// Trusted Git plumbing failed.
    #[error("trusted Git plumbing failed: {0}")]
    Git(String),
    /// Provider persistence failed.
    #[error("workspace persistence operation failed: {0}")]
    Persistence(String),
    /// Filesystem persistence failed.
    #[error("workspace filesystem operation failed: {0}")]
    Io(#[source] io::Error),
    /// Structured metadata serialization failed.
    #[error("workspace serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    /// Blocking workspace task failed to join.
    #[error("workspace task failed: {0}")]
    Task(#[source] tokio::task::JoinError),
    /// A platform-sized value could not be persisted.
    #[error("workspace integer conversion failed: {0}")]
    Integer(#[source] std::num::TryFromIntError),
}

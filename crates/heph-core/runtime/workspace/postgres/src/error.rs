//! Redacted workspace persistence errors.

use workspace_domain::WorkspaceRepositoryError;

pub fn error(error: impl std::fmt::Display) -> WorkspaceRepositoryError {
    WorkspaceRepositoryError::new(error.to_string())
}

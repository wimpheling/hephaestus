/// Redacted runtime Git credential failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeGitAuthorityError {
    /// Durable storage or scope reconstruction failed.
    #[error("runtime Git authority persistence failed")]
    Persistence,
    /// No exact live session/scope/credential matched.
    #[error("runtime Git authority is unavailable")]
    NotFound,
    /// Existing immutable issuance differs from this retry.
    #[error("runtime Git credential identity does not match")]
    IdentityMismatch,
    /// Presented password is not canonical runtime Git credential syntax.
    #[error("runtime Git credential is invalid")]
    InvalidCredential,
    /// Temporary bearer material is absent, expired, or corrupt.
    #[error("runtime Git credential handoff is unavailable")]
    HandoffUnavailable,
    /// An exact temporary envelope already exists.
    #[error("runtime Git credential handoff already exists")]
    HandoffExists,
}

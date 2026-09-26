/// Safe runtime authority issuance failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeAuthorityError {
    /// Durable storage was unavailable or rejected the operation.
    #[error("runtime authority persistence failed")]
    Persistence,
    /// The session does not exist or is deliberately hidden.
    #[error("runtime session is unavailable")]
    NotFound,
    /// Existing durable identity differs from the requested retry.
    #[error("runtime session identity does not match issuance request")]
    IdentityMismatch,
    /// A credential can no longer be delivered for this lifecycle state.
    #[error("runtime session is not pending handoff")]
    SessionNotPending,
    /// The acknowledgement generation differs from the issued generation.
    #[error("runtime session handoff generation does not match")]
    GenerationMismatch,
    /// The temporary host envelope is missing, expired, or cannot be opened.
    #[error("runtime credential handoff is unavailable")]
    HandoffUnavailable,
    /// An exact envelope already exists and must be redelivered instead.
    #[error("runtime credential handoff already exists")]
    HandoffExists,
}

/// Receive capability denial before repository mutation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReceivePolicyError {
    /// Resolved identity/scope state was incomplete, expired, or not writable.
    #[error("resolved runtime receive context is invalid")]
    InvalidResolvedContext,
    /// Host-owned hook authority was malformed or failed revalidation.
    #[error("runtime receive hook context is invalid")]
    InvalidHookContext,
    /// Runtime receives remain disabled unless an enforcement guard is wired.
    #[error("runtime receive policy guard is unavailable")]
    RuntimeGuardUnavailable,
    /// No trusted quarantine inspection was supplied for the runtime receive.
    #[error("trusted runtime receive proposal is unavailable")]
    TrustedProposalUnavailable,
    /// The proposal belongs to a different runtime session or snapshot.
    #[error("runtime receive identity binding does not match")]
    RuntimeBindingMismatch,
    /// Trusted transfer facts exceed the immutable capability limits.
    #[error("runtime receive transfer limit was exceeded")]
    TransferLimitExceeded,
    /// The atomic command batch is empty or too large.
    #[error("runtime receive ref-update count is invalid")]
    InvalidRefUpdateCount,
    /// Trigger-safe publication did not update exactly the snapshotted old
    /// commit.
    #[error("runtime receive expected parent does not match")]
    ExpectedParentMismatch,
    /// At least one ref transition or changed path is outside the scope.
    #[error("runtime receive update {update_index} is outside capability scope")]
    ScopeDenied {
        /// Zero-based command position; ref and path values are not exposed.
        update_index: usize,
    },
}

/// Failure from guarded authorization or the canonical mutation callback.
#[derive(Debug, thiserror::Error)]
pub enum GuardedReceiveError<E> {
    /// Authorization failed before canonical mutation.
    #[error(transparent)]
    Policy(#[from] ReceivePolicyError),
    /// The already-authorized canonical mutation failed.
    #[error("canonical Git receive mutation failed")]
    Mutation(E),
}

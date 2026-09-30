use crate::BuildRepositoryError;

/// Stable non-sensitive build execution failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BuildExecutionError {
    /// Static host configuration is unsafe.
    #[error("isolated build configuration is invalid")]
    InvalidConfiguration,
    /// Build request or exact configuration is unavailable.
    #[error("isolated build request is unavailable")]
    Unavailable,
    /// A durable execution already owns this build.
    #[error("isolated build request is already claimed")]
    AlreadyClaimed,
    /// The original requester no longer has permission to execute this build.
    #[error("isolated build execution is not authorized")]
    Unauthorized,
    /// The authorization provider could not make a safe decision.
    #[error("isolated build authorization failed")]
    Authorization,
    /// Stored build state is malformed.
    #[error("isolated build durable state is invalid")]
    StoredState,
    /// Build source repository path is unsafe.
    #[error("isolated build repository is unsafe")]
    UnsafeRepository,
    /// Transient workspace path is unsafe.
    #[error("isolated build workspace is unsafe")]
    UnsafeWorkspace,
    /// Source tree exceeds a platform bound.
    #[error("isolated build source exceeds a platform bound")]
    SourceQuota,
    /// Git tree encoding or path is invalid.
    #[error("isolated build source tree is invalid")]
    InvalidGitTree,
    /// Source contains an unsupported object such as a symlink or submodule.
    #[error("isolated build source contains an unsupported object")]
    UnsupportedSourceObject,
    /// Trusted Git inspection failed.
    #[error("isolated build Git inspection failed")]
    Git,
    /// The immutable OCI image selected for this build is not materialized on
    /// this worker.
    #[error("isolated build image is unavailable")]
    ImageUnavailable,
    /// Selected build network policy is not supported.
    #[error("isolated build network policy is denied")]
    NetworkDenied,
    /// VM provisioning, start, or monitoring failed.
    #[error("isolated build VM operation failed")]
    Vm,
    /// VM cleanup could not be confirmed, so import was not attempted.
    #[error("isolated build VM cleanup is incomplete")]
    VmCleanup,
    /// Guest exited unsuccessfully or timed out.
    #[error("isolated build guest failed")]
    GuestFailed,
    /// A verification rebuild produced a different artifact manifest.
    #[error("verification rebuild produced a different artifact manifest")]
    VerificationMismatch,
    /// Sealed output includes an unsafe object.
    #[error("isolated build output is unsafe")]
    UnsafeOutput,
    /// Guest emitted a path not declared by the immutable build contract.
    #[error("isolated build produced an undeclared output")]
    UndeclaredOutput,
    /// A required declared output was absent.
    #[error("isolated build omitted a declared output")]
    MissingOutput,
    /// Safe one-way artifact import failed.
    #[error(transparent)]
    Artifact(#[from] release_artifact_store::ArtifactStoreError),
    /// Draft release construction failed after import.
    #[error("isolated build release finalization failed")]
    Release,
    /// Durable persistence failed.
    #[error(transparent)]
    Database(BuildRepositoryError),
    /// A validated release value could not be reconstructed.
    #[error(transparent)]
    ReleaseValue(#[from] release_domain::ReleaseValueError),
    /// A blocking worker failed to join.
    #[error("isolated build worker failed")]
    WorkerJoin,
    /// Host filesystem operation failed.
    #[error("isolated build filesystem operation failed")]
    Filesystem,
}

impl From<BuildRepositoryError> for BuildExecutionError {
    fn from(error: BuildRepositoryError) -> Self {
        match error {
            BuildRepositoryError::Unavailable => Self::Unavailable,
            BuildRepositoryError::AlreadyClaimed => Self::AlreadyClaimed,
            BuildRepositoryError::Unauthorized => Self::Unauthorized,
            BuildRepositoryError::Authorization => Self::Authorization,
            BuildRepositoryError::InvalidData => Self::StoredState,
            other => Self::Database(other),
        }
    }
}

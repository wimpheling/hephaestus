use recipe_application::DeploymentError;
use volume_trait::VolumeError;

/// One prepared birth failure; no timeout or missing row proves `NotApplied`.
#[derive(Debug, thiserror::Error)]
pub enum OwnedVolumeCreateError {
    /// Fresh preparation/claim checks failed before the protected provider effect.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// The original claim remains held, even when ambiguity marking is denied.
    #[error("prepared owned volume remains held: {failure}")]
    Held {
        /// Actual primary error, never overwritten by secondary bookkeeping denial.
        #[source]
        failure: OwnedVolumeCreateFailure,
        /// Failure to record protected ambiguity under the current actor.
        ambiguity_error: Option<DeploymentError>,
    },
    /// Detached completion may still commit; caller must retry exact original context.
    #[error("first completion outcome is unknown; original claim remains protected")]
    CompletionUnknown,
}
/// Closed provider/completion stages, with no client outcome or proof token.
#[derive(Debug, thiserror::Error)]
pub enum OwnedVolumeCreateFailure {
    /// Real metadata/provider verification failure, possibly after IO.
    #[error(transparent)]
    Volume(#[from] VolumeError),
    /// Fresh actor, protected recorder or actor completion failed.
    #[error(transparent)]
    Completion(#[from] DeploymentError),
}

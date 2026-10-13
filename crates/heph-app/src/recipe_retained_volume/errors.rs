use recipe_application::DeploymentError;
use volume_trait::VolumeError;

/// Retained verification preserves primary failure and actual recovery denials.
#[derive(Debug, thiserror::Error)]
pub enum RetainedVolumeVerificationError {
    /// Fresh current actor preparation failed before protected completion.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// Retained worker or actual physical custody could not be qualified.
    #[error(transparent)]
    Volume(#[from] VolumeError),
    /// Full original birth, provider scope or exact new claim differs.
    #[error("retained volume verification correlation differs")]
    Correlation,
    /// Protected owner may still be running; retry the exact original new claim.
    #[error("retained volume completion is unknown; retry with current authority")]
    CompletionUnknown,
    /// No exact pair could be resolved under current authority; original claim stays held.
    #[error("retained volume verification remains held: {failure}")]
    Held {
        /// Original physical, worker or atomic actor failure.
        #[source]
        failure: RetainedVolumeVerificationFailure,
        /// Freshly authorized exact-pair reread failure, if any.
        recovery_error: Option<DeploymentError>,
        /// Current actor could not record ambiguity, if any.
        ambiguity_error: Option<DeploymentError>,
        /// Actual guard remains in an inspectable detached owner awaiting fresh recovery.
        custody_retained: bool,
    },
    /// A pending owner's retry could not resolve the original atomic actor outcome.
    #[error("retained volume completion remains unknown; actual custody is retained")]
    Pending {
        /// Current actor could not establish an exact pair or a closed pending state.
        #[source]
        recovery_error: DeploymentError,
        /// Current actor could not record ambiguity, if any.
        ambiguity_error: Option<DeploymentError>,
    },
}

/// Closed verification/completion failure; never absence or a client proof token.
#[derive(Debug, thiserror::Error)]
pub enum RetainedVolumeVerificationFailure {
    /// Actual retained worker or native custody validation failed.
    #[error(transparent)]
    Volume(#[from] VolumeError),
    /// Current actor, genuine worker receipt or atomic completion failed.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
}

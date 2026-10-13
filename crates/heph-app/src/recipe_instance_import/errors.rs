use recipe_application::DeploymentError;
use release_postgres::ReleaseServiceError;

/// Backend import failure, preserving the primary error when holding also fails.
#[derive(Debug, thiserror::Error)]
pub enum InstanceImportError {
    /// Preparation, authority or claim validation failed before the service call.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// A committed claim remains unresolved; absence is never inferred.
    #[error("prepared instance import remains held: {failure}")]
    Held {
        /// Actual primary failure at the service/completion boundary.
        #[source]
        failure: InstanceImportFailure,
        /// Secondary failure to record ambiguity, without replacing the primary.
        ambiguity_error: Option<DeploymentError>,
    },
}

/// Closed backend failure stages; no caller supplied outcome or receipt token.
#[derive(Debug, thiserror::Error)]
pub enum InstanceImportFailure {
    /// The actual release service returned an error, possibly after uncertain IO.
    #[error(transparent)]
    Release(#[from] Box<ReleaseServiceError>),
    /// Trusted recording or actor completion failed after real service success.
    #[error(transparent)]
    Completion(#[from] DeploymentError),
    /// The successful service result contradicts the checked frozen instance ID.
    #[error("release service returned a conflicting instance identity")]
    IdentityConflict,
}

/// Bounded deployment planning or repository failure.
#[derive(Debug, thiserror::Error)]
pub enum DeploymentError {
    /// A stable identity cannot be nil or malformed.
    #[error("invalid deployment identifier")]
    InvalidIdentifier,
    /// Deployment keys must be bounded, nonempty, and contain no controls.
    #[error("invalid deployment key")]
    InvalidKey,
    /// Parsed declaration and resolved graph have different immutable origins.
    #[error("declaration and resolved recipe do not match")]
    IntentMismatch,
    /// Immutable input changed under a previously admitted identity.
    #[error("deployment input conflicts with recorded intent")]
    InputConflict,
    /// The bounded recipe contract rejected the input.
    #[error("recipe intent is invalid")]
    Recipe(#[from] recipe_domain::RecipeError),
    /// Canonical intent serialization failed.
    #[error("deployment serialization failed")]
    Serialization,
    /// A persisted lifecycle value is outside the closed vocabulary.
    #[error("unknown deployment state")]
    UnknownState,
    /// An effect is inconsistent with kind, ownership, or removal policy.
    #[error("resource action is not permitted by recorded intent")]
    InvalidAction,
    /// Current actor permissions do not permit this operation.
    #[error("deployment authorization denied")]
    AuthorizationDenied,
    /// The requested record is unavailable to this caller.
    #[error("deployment is unavailable")]
    Unavailable,
    /// A compare-and-swap version or fencing claim is stale.
    #[error("deployment claim is stale")]
    StaleClaim,
    /// An ambiguous provider result must be reconciled before another effect.
    #[error("deployment resource requires reconciliation")]
    ReconciliationRequired,
    /// The durable adapter failed without exposing provider details.
    #[error("deployment repository failed")]
    Repository,
}

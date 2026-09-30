use capability_domain::CapabilityError;

/// Stable non-sensitive release service failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReleaseServiceError {
    /// Exact authorization was denied.
    #[error("release or instance command is not authorized")]
    AuthorizationDenied,
    /// Referenced build/release/agent/instance is unavailable.
    #[error("release or instance authority is unavailable")]
    Unavailable,
    /// Build is not at its sealed import boundary.
    #[error("build is not ready for immutable artifact import")]
    BuildNotImporting,
    /// No valid reusable configuration exists at the exact build commit.
    #[error("reusable release configuration is missing")]
    ReusableConfigurationMissing,
    /// Artifact set is empty.
    #[error("release artifact manifest is incomplete")]
    IncompleteArtifacts,
    /// Artifact metadata violates bounds.
    #[error("release artifact metadata is invalid")]
    InvalidArtifact,
    /// Typed parameter diagnostics prevented a revision.
    #[error("agent instance parameters are invalid")]
    InvalidParameters(Vec<release_domain::ParameterDiagnostic>),
    /// Stored JSON/provenance violates a domain invariant.
    #[error("stored release data is invalid")]
    InvalidStoredData,
    /// Expected active revision lost its compare-and-swap race.
    #[error("agent instance revision changed concurrently")]
    StaleInstanceRevision,
    /// One carried secret binding is no longer live and bindable.
    #[error("agent instance secret binding is unavailable")]
    SecretBindingUnavailable,
    /// Capability declaration or binding violates the immutable release ceiling.
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    /// Typed Git ceiling or attenuation is malformed.
    #[error("runtime Git authority is invalid")]
    GitCapability(#[from] git_capability_domain::GitCapabilityError),
    /// Capability selection is duplicated, unknown, or otherwise malformed.
    #[error("agent instance capability binding is invalid")]
    InvalidCapabilityBinding,
    /// Exact resource is missing, outside the project, or not implemented.
    #[error("agent instance capability resource is unavailable")]
    CapabilityResourceUnavailable,
    /// Another update already closed the instance run gate.
    #[error("agent instance already has an active update")]
    ConcurrentUpdate,
    /// Candidate export belongs to another source agent family.
    #[error("agent update candidate belongs to another family")]
    AgentFamilyMismatch,
    /// Update or instance is not at the requested durable boundary.
    #[error("agent update lifecycle does not permit this operation")]
    InvalidUpdateLifecycle,
    /// Pre-gate normal requests or runs have not drained.
    #[error("agent update is waiting for normal runs to drain")]
    UpdateDrainPending,
    /// A concurrent update-hook admission won the durable run identity race.
    #[error("agent update hook admission raced another generation")]
    UpdateAdmissionGenerationRace,
    /// Persistent state volume is not ready for an exclusive update lease.
    #[error("agent update state volume is unavailable")]
    UpdateVolumeUnavailable,
    /// Update lease host or expiry violates bounds.
    #[error("agent update volume lease is invalid")]
    InvalidUpdateLease,
    /// Hook exit result is malformed.
    #[error("agent update hook result is invalid")]
    InvalidHookResult,
    /// One command identity was reused for another operation.
    #[error("release command idempotency identity conflicts")]
    IdempotencyConflict,
    /// Release domain validation failure.
    #[error(transparent)]
    Domain(#[from] release_domain::ReleaseValueError),
    /// Authorization provider failure.
    #[error(transparent)]
    Authorization(#[from] authz_domain::AuthzError),
    /// JSON serialization failure.
    #[error("release configuration serialization failed")]
    Serialization(#[from] serde_json::Error),
    /// Database failure.
    #[error("release persistence failed")]
    Database(#[from] sqlx::Error),
}

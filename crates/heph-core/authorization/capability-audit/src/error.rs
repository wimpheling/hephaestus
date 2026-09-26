use std::error::Error;

/// Provider-neutral capability audit failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CapabilityAuditError {
    /// A reason code was not canonical lower snake case.
    #[error("capability audit reason code is invalid")]
    InvalidReasonCode,
    /// The requested page size was outside the supported bound.
    #[error("capability audit page size is invalid")]
    InvalidPageSize,
    /// Persisted evidence was malformed or inconsistent.
    #[error("capability audit evidence is invalid")]
    InvalidEvidence,
    /// The caller cannot inspect the run, or the run is deliberately hidden.
    #[error("capability audit evidence is unavailable")]
    Unavailable,
    /// The configured provider failed.
    #[error("capability audit provider failed: {0}")]
    Provider(#[source] Box<dyn Error + Send + Sync>),
}

impl CapabilityAuditError {
    /// Wraps a provider failure without exposing its concrete type.
    #[must_use]
    pub fn provider(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Provider(Box::new(error))
    }
}

impl PartialEq for CapabilityAuditError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Eq for CapabilityAuditError {}

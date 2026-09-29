use std::error::Error;

use super::VmId;

/// An error returned by a VM provider or instance.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VmError {
    /// A VM with the requested identifier already exists.
    #[error("VM already exists: {0:?}")]
    AlreadyExists(VmId),

    /// A field in the requested VM specification is invalid.
    #[error("invalid VM specification field {field:?}: {reason}")]
    InvalidSpec {
        /// Field path within the VM specification.
        field: String,
        /// Human-readable validation failure.
        reason: String,
    },

    /// The selected provider does not support a requested feature.
    #[error("provider {provider:?} does not support {feature}")]
    Unsupported {
        /// Unsupported provider-neutral feature.
        feature: String,
        /// Provider that rejected the feature.
        provider: String,
    },

    /// The VM was destroyed without producing an exit status.
    #[error("VM was destroyed before it produced an exit status")]
    Destroyed,

    /// A required runtime resource is temporarily unavailable.
    #[error("resource {resource:?} is unavailable: {reason}")]
    Unavailable {
        /// Resource that could not be acquired.
        resource: String,
        /// Human-readable reason the resource is unavailable.
        reason: String,
    },

    /// The requested operation is not valid in the VM's current lifecycle state.
    #[error("VM is in an invalid state: {0}")]
    InvalidState(&'static str),

    /// An unexpected provider-specific failure.
    #[error("unexpected {provider} provider error ({code}): {source}")]
    Provider {
        /// Provider that failed.
        provider: String,
        /// Stable provider-specific diagnostic code.
        code: String,
        /// Original error returned by the provider backend.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },
}

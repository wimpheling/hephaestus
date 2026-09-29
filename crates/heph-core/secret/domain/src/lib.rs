//! Provider-neutral secret ownership, delegation, binding, and lease contracts.
//!
//! This crate deliberately has no plaintext retrieval contract. [`SecretValue`]
//! can enter a storage implementation, but its formatting and serialization
//! representations are always redacted.

/// Maximum accepted plaintext size for one secret version.
pub const MAX_SECRET_VALUE_BYTES: usize = 65_536;
/// Maximum number of destination constraints on a grant or binding.
pub const MAX_DESTINATIONS: usize = 32;

mod diagnostics;
mod entities;
mod identifiers;
mod policy;
mod redaction;

pub use diagnostics::{
    SecretCommandKey, SecretDiagnostic, SecretDiagnosticCode, SecretValueError,
    validate_tenant_boundary,
};
pub use entities::{
    AgentSecretBinding, GatewaySecretLease, Secret, SecretGrant, SecretImport, SecretLease,
    SecretVersion,
};
pub use identifiers::{
    AgentSecretBindingId, GatewaySecretBindingId, GatewaySecretLeaseId, SecretAlias, SecretGrantId,
    SecretId, SecretImportId, SecretLeaseId, SecretName, SecretRuntimeSessionId, SecretSlotKey,
    SecretVersionId,
};
pub use policy::{
    AuthorityStatus, DeliveryMode, ExecutionPhase, SecretOwner, SecretStatus, SecretTarget,
    SecretUsePolicy,
};
pub use redaction::{OpaqueRuntimeCredential, SecretValue};

#[cfg(test)]
#[path = "tests/secret_domain.rs"]
mod tests;

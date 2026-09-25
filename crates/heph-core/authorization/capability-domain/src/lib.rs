//! Provider-neutral capability requirements, bindings, and runtime ceilings.
//!
//! These values describe immutable authority. Storage adapters and live
//! authorization providers remain responsible for persistence, revocation,
//! tenant boundaries, and current allow/deny decisions.

mod capabilities;
mod credentials;
mod errors;
mod ids;
mod requirements;
mod workload;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use capabilities::{CapabilityOperation, CapabilityResourceKind, CapabilitySlotKey};
pub use credentials::{
    AuthorityHash, RuntimeAuthority, RuntimeCredential, RuntimeCredentialGeneration,
    RuntimeCredentialHash, RuntimeSessionStatus,
};
pub use errors::CapabilityError;
pub use ids::{
    AuthorizationSnapshotId, CapabilityBindingId, CapabilityRequirementId, GatewayInvocationId,
    RuntimeSessionId,
};
pub use requirements::{CapabilityBinding, CapabilityRequirement, CapabilityResource};
pub use workload::{
    AuthorizationSnapshot, RuntimeInvocation, RuntimeSessionIdentity, WorkloadKind,
    WorkloadPrincipal,
};

/// Maximum length of a symbolic capability slot key.
pub const MAX_CAPABILITY_SLOT_KEY_BYTES: usize = 64;
/// Maximum number of operations declared by or granted to one slot.
pub const MAX_OPERATIONS_PER_CAPABILITY: usize = 32;
/// Size of a canonical authority hash in bytes.
pub const AUTHORITY_HASH_BYTES: usize = 32;
/// Size of a runtime bearer credential in bytes.
pub const RUNTIME_CREDENTIAL_BYTES: usize = 32;

//! Forge-owned OCI registry control-plane contracts.
//!
//! Zot remains authoritative for OCI content. This crate models only the
//! durable ownership, immutable identity, verification, and approval decisions
//! Hephaestus must make before content can be exposed or executed.

mod authority;
mod errors;
mod inventory;
mod namespace;
mod oci;
mod publication;
mod supply;

pub use authority::{RegistryAuthority, Sha256Digest};
pub use errors::{PublicationLifecycleError, RegistryConsumptionError, RegistryValueError};
pub use inventory::{
    MAX_REGISTRY_INVENTORY_ENTRIES, RegistryInventory, RegistryInventoryDocument,
    RegistryInventoryEntry, RegistryNotificationBacklog, RegistryOperationalMetrics,
    RegistryRetentionReport, RegistryRetentionReportError, RegistryRetentionReportMode,
    RegistryRetentionRoot, RegistryRetentionRootKind, RegistryRetentionSchemaScope,
    RegistryRetentionSnapshot,
};
pub use namespace::{
    NamespaceClaim, PlatformImageKey, PublicationIntentId, RegistryNamespace, RegistryOwner,
    RegistryOwnershipError,
};
pub use oci::{ImmutableManifestReference, OciDescriptor, OciMediaType, PlatformDescriptor};
pub use publication::PublicationIntent;
pub use supply::{
    PolicyVersion, PublicationState, SupplyChainEvidence, SupplyChainPolicy, SupplyChainReferrer,
    SupplyChainReferrerKind, VerifiedPublication,
};

#[cfg(test)]
mod tests {
    mod identity;
    mod publication;
    mod retention;
    mod support;
}

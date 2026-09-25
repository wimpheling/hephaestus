//! Local exact-runtime materialization for immutable release artifacts and
//! host-generated run context.
//!
//! Repository paths are used only inside a fresh administrator-owned staging
//! tree. Canonical artifact storage is addressed exclusively by opaque UUID,
//! and every object is rehashed while copied into the non-reusable run tree.

pub(crate) const MAX_RUNTIME_ARTIFACTS: usize = 4_096;
pub(crate) const MAX_RUNTIME_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const RELEASE_TAG_PREFIX: &str = "rel";
pub(crate) const CONTEXT_TAG_PREFIX: &str = "ctx";
pub(crate) const PREVIOUS_RELEASE_TAG_PREFIX: &str = "old";
pub(crate) const GATEWAY_RELEASE_TAG_PREFIX: &str = "gwr";
pub(crate) const GATEWAY_SERVICE_NAMESPACE: &str = "gateway-services";
pub(crate) const GATEWAY_SERVICE_STAGING_PREFIX: &str = ".prepare-";
pub(crate) const GATEWAY_SERVICE_METADATA: &str = "identity.json";
pub(crate) const GATEWAY_SERVICE_SCHEMA_VERSION: u8 = 1;
pub(crate) const MAX_GATEWAY_SERVICE_METADATA_BYTES: u64 = 1_024;

mod artifacts;
mod context;
mod filesystem;
mod gateway;
mod manager;
mod recovery;
mod release;
#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
mod types;

pub use types::{
    GatewayServiceIdentity, GatewayServiceInstanceRecord, LocalGatewayReleaseRuntime,
    LocalRunRuntimeConfig, LocalRunRuntimeManager,
};

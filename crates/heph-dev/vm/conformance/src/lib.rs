//! Reusable behavioral conformance tests for [`vm_trait`] providers.
//!
//! Provider crates supply specifications and cleanup assertions through
//! [`ProviderHarness`]. These tests observe only the public VM contract;
//! backend implementation details belong in provider-local suites.

mod api;
mod invalid;
mod lifecycle;
mod macros;
mod optional;
mod paths;
mod suite;
mod support;

pub use api::{ProviderHarness, TestCapabilities};
pub use invalid::invalid_core_specs_are_typed;
pub use lifecycle::{
    caller_owned_paths_survive_destroy, concurrent_start_is_shared, destroy_before_start_is_typed,
    destroy_running_is_idempotent, identifiers_are_unique_and_reusable, provision_is_stopped,
    stop_is_idempotent, wait_is_shared_and_cached,
};
pub use optional::ephemeral_ingress_is_resolved;
pub use suite::lifecycle_suite;

use super::{
    api::ProviderHarness,
    invalid::invalid_core_specs_are_typed,
    lifecycle::{
        caller_owned_paths_survive_destroy, concurrent_start_is_shared,
        destroy_before_start_is_typed, destroy_running_is_idempotent,
        identifiers_are_unique_and_reusable, provision_is_stopped, stop_is_idempotent,
        wait_is_shared_and_cached,
    },
    optional::ephemeral_ingress_is_resolved,
};

/// Runs every mandatory provider-neutral lifecycle check sequentially.
///
/// # Panics
///
/// Panics when any constituent conformance check fails.
pub async fn lifecycle_suite(harness: &impl ProviderHarness) {
    provision_is_stopped(harness).await;
    concurrent_start_is_shared(harness).await;
    wait_is_shared_and_cached(harness).await;
    destroy_before_start_is_typed(harness).await;
    destroy_running_is_idempotent(harness).await;
    caller_owned_paths_survive_destroy(harness).await;
    stop_is_idempotent(harness).await;
    identifiers_are_unique_and_reusable(harness).await;
    ephemeral_ingress_is_resolved(harness).await;
    invalid_core_specs_are_typed(harness).await;
}

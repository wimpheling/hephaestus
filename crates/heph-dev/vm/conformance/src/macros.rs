/// The path must name a zero-argument function returning a type that
/// implements [`crate::ProviderHarness`].
#[macro_export]
macro_rules! provider_conformance_tests {
    ($factory:path) => {
        #[tokio::test]
        async fn conformance_provision_is_stopped() {
            $crate::provision_is_stopped(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_concurrent_start_is_shared() {
            $crate::concurrent_start_is_shared(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_wait_is_shared_and_cached() {
            $crate::wait_is_shared_and_cached(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_destroy_before_start_is_typed() {
            $crate::destroy_before_start_is_typed(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_destroy_running_is_idempotent() {
            $crate::destroy_running_is_idempotent(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_caller_owned_paths_survive_destroy() {
            $crate::caller_owned_paths_survive_destroy(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_stop_is_idempotent() {
            $crate::stop_is_idempotent(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_identifiers_are_unique_and_reusable() {
            $crate::identifiers_are_unique_and_reusable(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_ephemeral_ingress_is_resolved() {
            $crate::ephemeral_ingress_is_resolved(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_invalid_core_specs_are_typed() {
            $crate::invalid_core_specs_are_typed(&$factory()).await;
        }
    };
}

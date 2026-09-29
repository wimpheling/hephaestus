//! Connect RPC transport authentication and service composition.

mod artifact;
mod auth;
mod build;
mod error;
mod image_catalog;
// The event adapter is shared with the single outbound product-event adapter.
#[allow(clippy::redundant_pub_crate)]
pub(crate) mod event;
mod gateway;
mod identity;
mod instance;
mod organization;
mod pat;
mod project;
mod release;
mod repository;
mod repository_browser;
mod request;
mod run;
mod secret;

mod module_service;
mod module_support;

pub use auth::mediator_identity_middleware;
pub use auth::{
    BootstrapIdentity, MediatorAssertionError, MediatorAuthenticationState, MediatorAuthenticator,
    MediatorPrincipal, VerifiedMediatorSession,
};
pub use error::{RpcError, into_connect_error};

pub(crate) use module_service::{ApplicationDependencies, service};
// Keep the initialization error in the public RPC module API for callers that name it.
#[allow(unused_imports)]
pub(crate) use module_service::RpcInitializationError;
#[cfg(test)]
use module_support::ConnectReceiptError;
pub use module_support::mediator_signing_key;
pub(crate) use module_support::{
    DEFAULT_DEADLINE_SECONDS, GLOBAL_MAX_MESSAGE_BYTES, GLOBAL_MAX_REQUEST_BYTES,
    MAX_DEADLINE_SECONDS, STREAM_IDLE_SECONDS,
};
pub(crate) use module_support::{MutationReceipts, mutation_receipt};

#[cfg(test)]
mod tests {
    use super::{
        ConnectReceiptError, DEFAULT_DEADLINE_SECONDS, GLOBAL_MAX_MESSAGE_BYTES,
        GLOBAL_MAX_REQUEST_BYTES, MAX_DEADLINE_SECONDS, STREAM_IDLE_SECONDS,
    };
    use connectrpc_reflection::{
        Reflector, SERVER_REFLECTION_SERVICE_NAME, SERVER_REFLECTION_V1ALPHA_SERVICE_NAME,
    };
    use event_application::MutationReceiptError;
    use std::sync::Arc;

    #[test]
    fn checked_in_descriptors_expose_application_and_reflection_services() {
        let reflector = Reflector::from_descriptor_pool(Arc::new(
            rpc_proto::descriptor_pool().expect("checked-in descriptor pool"),
        ))
        .expect("reflection index");
        let services = reflector.service_names();
        assert!(
            services
                .iter()
                .any(|service| service == "hephaestus.identity.v1.IdentityService")
        );
        assert!(
            services
                .iter()
                .any(|service| service == SERVER_REFLECTION_SERVICE_NAME)
        );
        assert!(
            services
                .iter()
                .any(|service| service == SERVER_REFLECTION_V1ALPHA_SERVICE_NAME)
        );
    }

    #[test]
    fn transport_limits_leave_preview_envelope_headroom_and_bound_deadlines() {
        const MAX_ARTIFACT_PREVIEW_BYTES: usize = 1024 * 1024;
        const MAX_INSTANCE_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
        assert_eq!(GLOBAL_MAX_REQUEST_BYTES, MAX_ARTIFACT_PREVIEW_BYTES);
        assert_eq!(GLOBAL_MAX_MESSAGE_BYTES, MAX_INSTANCE_RESPONSE_BYTES);
        assert_eq!(MAX_DEADLINE_SECONDS, 60);
        assert_eq!(DEFAULT_DEADLINE_SECONDS, 30);
        assert_eq!(STREAM_IDLE_SECONDS, 10);
    }

    #[test]
    fn mutation_receipt_diagnostics_use_closed_redacted_error_classes() {
        let missing = ConnectReceiptError::Application(MutationReceiptError::Missing);
        assert_eq!(missing.error_class(), "missing");

        let provider = ConnectReceiptError::Application(MutationReceiptError::provider(
            std::io::Error::other("database password=should-not-be-logged"),
        ));
        assert_eq!(provider.error_class(), "provider-unavailable");
        assert_eq!(
            provider.to_string(),
            "mutation receipt application operation failed"
        );
        assert!(!provider.to_string().contains("should-not-be-logged"));

        let invalid = ConnectReceiptError::InvalidVersion;
        assert_eq!(invalid.error_class(), "invalid-version");
    }
}

//! `PostgreSQL` installation authority for repository-declared HTTP gateways.
//!
//! A caller supplies the exact repository manifest bytes from the release it
//! is installing. This adapter parses that source before opening its
//! transaction, authorizes project management in that transaction, and then
//! atomically installs only immutable declaration revisions and their routes.
//! It deliberately does not open a listener or derive provider configuration.

use authz_postgres::begin_actor_transaction;
use gateway_domain::{GatewayEdgeError, GatewayLimits, GatewayRouteBinding};
#[cfg(test)]
use http::Method;
#[cfg(test)]
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope,
};
#[cfg(test)]
use release_domain::ReleaseId;
#[cfg(test)]
use sha2::{Digest, Sha256};
#[cfg(test)]
use time::OffsetDateTime;
#[cfg(test)]
use uuid::Uuid;

mod edge_authority;
mod edge_invocations;
mod edge_recovery;
mod edge_release;
mod gateway_mailbox_payload;
mod gateway_mailbox_publisher;
mod release_artifacts;
mod release_resolver;
mod route_authority;
mod runtime_contract;
mod service_execution;
mod service_failure;
mod service_launch;
mod service_log_reader;
mod service_logs;
pub(crate) mod service_ownership;
mod service_targets;
mod service_vm;
pub(crate) mod ui_browser;

pub use edge_authority::PostgresGatewayEdgeAuthority;
#[cfg(test)]
pub(crate) use edge_invocations::desired_configuration_revision;
pub use edge_release::{
    GatewayReleaseArtifact, GatewayReleaseArtifactKind, GatewayReleaseMaterializer,
};
#[cfg(test)]
use gateway_domain::{Exposure, HttpMethod};
#[cfg(test)]
use gateway_mailbox_payload::validate_gateway_mailbox_payload;
pub use gateway_mailbox_publisher::{
    GatewayMailboxPublicationRequest, GatewayMailboxPublicationResult,
    PostgresGatewayMailboxPublisher,
};
#[cfg(test)]
use installer_declaration::{exposure_name, installation_hash, method_name, parse_manifest};
pub use installer_models::{
    GatewayInstallError, InstallGatewayManifest, InstallGatewayManifestResult, InstalledGateway,
    PostgresGatewayInstaller, PublishedGatewayRelease,
};
pub(crate) use management_configuration_validation::validate_secret_selections;
pub(crate) use management_helpers::{
    binding_command_key, binding_payload_hash, configure_payload_hash, load_mailbox_binding,
    secret_selection_hash, valid_gateway_producer, valid_gateway_slot, valid_header_name,
};
pub use management_models::{
    ConfigureGatewayRequest, ConfigureGatewayResult, GatewayConfigureError, GatewayIngressSummary,
    GatewayMailboxBindingSummary, GatewayMailboxPublicationSummary, GatewayManagementError,
    GatewayManagementRevision, GatewayManagementRoute, GatewayManagementSummary, GatewayPage,
    GatewaySecretSelection,
};
pub(crate) use management_rows::{
    ConfigureCommandRow, ConfigureRevisionRow, ConfigureRouteRow, GatewayIngressRow,
    GatewayMailboxBindingCommandRow, GatewayMailboxBindingRow, GatewayMailboxBindingTargetRow,
    GatewayMailboxPublicationManagementRow, GatewayRevisionRow, GatewayRouteRow, GatewaySummaryRow,
};
pub use management_service::PostgresGatewayManagement;
pub(crate) use release_artifacts::gateway_release_artifacts;
pub use release_resolver::PostgresGatewayReleaseResolver;
pub(crate) use route_authority::AcceptedInvocationRow;
#[cfg(test)]
use route_authority::active_route;
use route_authority::{canonical_request_path, route_matches};
pub(crate) use runtime_contract::{GatewayNetwork, GatewayRuntimeContract};
pub use service_execution::PostgresGatewayExecutionTargetResolver;
pub use service_failure::PostgresGatewayServiceFailureStore;
pub use service_launch::PostgresGatewayServiceLaunchResolver;
pub use service_log_reader::{GatewayServiceLogReaderError, PostgresGatewayServiceLogReader};
pub use service_logs::PostgresGatewayServiceLogStore;
pub use service_ownership::PostgresGatewayServiceOwnership;
pub use service_targets::PostgresGatewayServiceTargets;
pub(crate) use service_vm::service_vm_spec;

const SERVICE_RECOVERY_BATCH_SIZE: i64 = 128;

mod installer_command_ledger;
mod installer_commands;
mod installer_declaration;
mod installer_models;
mod management_commands;
mod management_configuration;
mod management_configuration_validation;
mod management_helpers;
mod management_models;
mod management_queries;
mod management_rows;
mod management_service;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{BTreeMap, BTreeSet},
        time::Duration,
    };

    #[test]
    fn parses_only_validated_repository_gateway_source() {
        let source = br#"
version = 1

[[gateways]]
name = "telegram"
agent_name = "telegram-handler"
handler_contract = "http.v1"
exposure = "public"
parameters = {}

[[gateways.routes]]
path = "/telegram"
methods = ["POST"]
"#;
        let parsed = parse_manifest(source).expect("valid repository source");
        assert_eq!(parsed.gateways.len(), 1);
        assert!(parse_manifest(b"version = 2").is_err());
        assert!(parse_manifest(&vec![b'x'; 1_048_577]).is_err());
    }

    #[test]
    fn serializes_only_the_bounded_http_vocabulary() {
        assert_eq!(method_name(HttpMethod::Patch), "PATCH");
        assert_eq!(
            exposure_name(Exposure::HephAuthenticated),
            "heph_authenticated"
        );
    }

    #[test]
    fn mailbox_binding_input_is_strictly_bounded_before_persistence() {
        assert!(valid_gateway_slot("recipe-events_2"));
        assert!(!valid_gateway_slot("Recipe-events"));
        assert!(!valid_gateway_slot("-recipe-events"));
        assert!(!valid_gateway_slot(&"a".repeat(65)));
        assert!(valid_gateway_producer("telegram-relay/v1"));
        assert!(!valid_gateway_producer(" producer"));
        assert!(!valid_gateway_producer("producer\nnext"));
        assert!(!valid_gateway_producer(&"p".repeat(129)));
    }

    #[test]
    fn publication_inspection_maps_only_joined_lifecycle_provenance() {
        let now = OffsetDateTime::now_utc();
        let snapshot_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let summary =
            GatewayMailboxPublicationSummary::from(GatewayMailboxPublicationManagementRow {
                id: Uuid::new_v4(),
                invocation_id: Uuid::new_v4(),
                gateway_revision_id: Uuid::new_v4(),
                binding_id: Some(Uuid::new_v4()),
                grant_id: Some(Uuid::new_v4()),
                mailbox_id: Some(Uuid::new_v4()),
                event_id: Some(Uuid::new_v4()),
                slot_key: String::from("recipe-events"),
                outcome: String::from("accepted"),
                accepted_at: now,
                settled_at: now,
                authorization_snapshot_id: Some(snapshot_id),
                snapshot_binding_ordinal: Some(3),
                delivery_disposition: Some(String::from("delivered")),
                delivery_attempt_count: Some(1),
                delivery_terminal_at: Some(now),
                delivery_attempt_id: Some(attempt_id),
                run_id: Some(run_id),
                run_state: Some(String::from("succeeded")),
                run_outcome: Some(String::from("succeeded")),
            });
        assert_eq!(summary.authorization_snapshot_id, Some(snapshot_id));
        assert_eq!(summary.snapshot_binding_ordinal, Some(3));
        assert_eq!(summary.delivery_disposition.as_deref(), Some("delivered"));
        assert_eq!(summary.delivery_attempt_id, Some(attempt_id));
        assert_eq!(summary.run_id, Some(run_id));
        assert_eq!(summary.run_outcome.as_deref(), Some("succeeded"));
    }

    #[test]
    fn gateway_mailbox_payload_validation_preserves_an_empty_body() {
        let body = Vec::new();
        let hash: [u8; 32] = Sha256::digest(&body).into();
        let envelope = MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/empty").expect("route"),
            BTreeMap::new(),
            ContentMetadata::new(
                BodyReference::new(BodyReferenceId::new(), 0, hash).expect("zero body"),
                None,
                Some(String::from("identity")),
            )
            .expect("content"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("envelope");
        let request = GatewayMailboxPublicationRequest {
            runtime_session_id: Uuid::new_v4(),
            invocation_id: Uuid::new_v4(),
            slot_key: String::from("recipe-events"),
            deduplication_key: DeduplicationKey::parse("empty-body").expect("key"),
            envelope,
            encoded_body: body,
            decoded_length: 0,
        };

        assert!(validate_gateway_mailbox_payload(&request).is_ok());
    }

    fn edge_limits() -> GatewayLimits {
        GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 1024,
            execution_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn edge_resolution_selects_the_longest_exact_path_segment() {
        let short = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("telegram"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        let nested = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("telegram/updates"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        assert!(route_matches(&short, "/gateway/telegram"));
        assert!(route_matches(&nested, "/gateway/telegram/updates"));
        assert!(!route_matches(&short, "/gateway/telegram-bot"));
        assert_eq!(
            canonical_request_path("/gateway/telegram/updates?offset=1").expect("path"),
            "/gateway/telegram/updates"
        );
    }

    #[test]
    fn edge_route_conversion_rejects_unknown_persisted_method() {
        let row = route_authority::ActiveRouteRow {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path: String::from("/telegram"),
            methods: vec![String::from("CONNECT")],
            exposure: String::from("public"),
        };
        assert!(active_route(row, edge_limits()).is_err());
    }

    #[test]
    fn desired_configuration_revision_is_order_independent_and_tracks_cutover() {
        let first = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("first"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        let second = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("second"),
            methods: BTreeSet::from([Method::GET]),
            limits: edge_limits(),
        };
        assert_eq!(
            desired_configuration_revision(&[first.clone(), second.clone()]),
            desired_configuration_revision(&[second.clone(), first.clone()])
        );
        let before_cutover = desired_configuration_revision(&[first.clone(), second.clone()]);
        let replacement = GatewayRouteBinding {
            gateway_revision_id: Uuid::new_v4(),
            ..second
        };
        assert_ne!(
            before_cutover,
            desired_configuration_revision(&[first, replacement])
        );
    }

    #[test]
    fn installation_hash_is_stable_per_release_and_agent() {
        let declaration = [7_u8; 32];
        let release = ReleaseId::new();
        let agent = Uuid::new_v4();
        assert_eq!(
            installation_hash(declaration, Some(release), agent),
            installation_hash(declaration, Some(release), agent)
        );
        assert_ne!(
            installation_hash(declaration, Some(release), agent),
            installation_hash(declaration, Some(ReleaseId::new()), agent)
        );
        assert_ne!(
            installation_hash(declaration, Some(release), agent),
            installation_hash(declaration, Some(release), Uuid::new_v4())
        );
    }
}

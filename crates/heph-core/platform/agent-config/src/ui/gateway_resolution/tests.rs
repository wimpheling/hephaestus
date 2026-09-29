//! Behavioral tests for UI gateway resolution.

use super::{
    GatewayReferenceKind, GatewayResolutionError, ReleaseAgentBinding, resolve_gateway_uis,
};
use crate::{parse_repository_gateways, parse_repository_uis};
use release_domain::{AgentKey, ReleaseAgentId};
use uuid::Uuid;

const UI: &str = r#"
version = 1

[[uis]]
key = "assistant"
scope = "project"
label = "Assistant"
icon = "chat"
presentation = "iframe"
route_base = "assistant"
ui_kit_version = 1
cache = "no_store"

[[uis.apis]]
key = "service-api"
gateway_name = "ui-service"
method = "GET"
route = "/service/api"

[uis.content]
kind = "managed_service"
gateway_name = "ui-service"
route = "/service/ui"
entrypoint = "index.html"
"#;

const GATEWAYS: &str = r#"
version = 1

[[gateways]]
name = "ui-service"
agent_name = "ui-service-agent"
handler_contract = "http.service.v1"
exposure = "heph_authenticated"

[gateways.service]
loopback_port = 8080
readiness_path = "/ready"
health_path = "/health"

[[gateways.routes]]
path = "/service"
methods = ["GET"]
"#;

fn id(value: u128) -> ReleaseAgentId {
    ReleaseAgentId::from_uuid(Uuid::from_u128(value))
}

fn binding(key: &str, value: u128) -> ReleaseAgentBinding {
    ReleaseAgentBinding {
        agent_key: AgentKey::parse(key).expect("agent key"),
        release_agent_id: id(value),
    }
}

fn parsed() -> (
    crate::ui::RepositoryUisConfig,
    crate::RepositoryGatewaysConfig,
) {
    (
        parse_repository_uis(UI.as_bytes())
            .config
            .expect("valid UI"),
        parse_repository_gateways(GATEWAYS.as_bytes())
            .config
            .expect("valid gateways"),
    )
}

#[test]
fn retains_exact_agent_id_for_managed_and_api_bindings() {
    let (uis, gateways) = parsed();
    let result = resolve_gateway_uis(&uis, Some(&gateways), &[binding("ui-service-agent", 1)])
        .expect("gateway bindings");
    assert_eq!(
        result.uis[0]
            .managed_service
            .as_ref()
            .expect("managed service")
            .release_agent_id,
        id(1)
    );
    assert_eq!(result.uis[0].apis[0].release_agent_id, id(1));
}

#[test]
fn missing_agent_is_rejected_without_gateway_name_echo() {
    let (uis, gateways) = parsed();
    let error = resolve_gateway_uis(&uis, Some(&gateways), &[binding("other-agent", 1)])
        .expect_err("missing agent");
    assert_eq!(
        error,
        GatewayResolutionError::MissingAgent {
            ui_index: 0,
            reference_kind: GatewayReferenceKind::ManagedService,
            reference_index: 0,
        }
    );
    assert!(!error.to_string().contains("ui-service"));
    assert!(!error.to_string().contains("ui-service-agent"));
}

#[test]
fn static_ui_needs_no_gateway_manifest() {
    let uis = parse_repository_uis(
            "version = 1\n\n[[uis]]\nkey = \"docs\"\nscope = \"global\"\nlabel = \"Docs\"\nicon = \"book\"\npresentation = \"iframe\"\nroute_base = \"docs\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n".as_bytes(),
        )
        .config
        .expect("valid static UI");
    let result = resolve_gateway_uis(&uis, None, &[]).expect("static UI");
    assert!(result.uis[0].managed_service.is_none());
    assert!(result.uis[0].apis.is_empty());
}

#[test]
fn one_agent_can_back_two_distinct_gateway_names() {
    let ui = parse_repository_uis(
            "version = 1\n\n[[uis]]\nkey = \"api-ui\"\nscope = \"global\"\nlabel = \"API UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"api-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[[uis.apis]]\nkey = \"one\"\ngateway_name = \"one\"\nmethod = \"GET\"\nroute = \"/one\"\n\n[[uis.apis]]\nkey = \"two\"\ngateway_name = \"two\"\nmethod = \"GET\"\nroute = \"/two\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n".as_bytes(),
        )
        .config
        .expect("valid UI");
    let gateways = parse_repository_gateways(
            "version = 1\n\n[[gateways]]\nname = \"one\"\nagent_name = \"shared-agent\"\nhandler_contract = \"http.v1\"\nexposure = \"heph_authenticated\"\n\n[[gateways.routes]]\npath = \"/one\"\nmethods = [\"GET\"]\n\n[[gateways]]\nname = \"two\"\nagent_name = \"shared-agent\"\nhandler_contract = \"http.v1\"\nexposure = \"heph_authenticated\"\n\n[[gateways.routes]]\npath = \"/two\"\nmethods = [\"GET\"]\n".as_bytes(),
        )
        .config
        .expect("valid gateways");
    let result = resolve_gateway_uis(&ui, Some(&gateways), &[binding("shared-agent", 3)])
        .expect("shared agent bindings");
    assert_eq!(result.uis[0].apis[0].release_agent_id, id(3));
    assert_eq!(result.uis[0].apis[1].release_agent_id, id(3));
}

#[test]
fn duplicate_agent_keys_and_ids_are_rejected() {
    let (uis, gateways) = parsed();
    let error = resolve_gateway_uis(
        &uis,
        Some(&gateways),
        &[
            binding("ui-service-agent", 1),
            binding("ui-service-agent", 2),
        ],
    )
    .expect_err("duplicate key");
    assert!(matches!(
        error,
        GatewayResolutionError::DuplicateAgentKey { .. }
    ));

    let error = resolve_gateway_uis(
        &uis,
        Some(&gateways),
        &[binding("ui-service-agent", 1), binding("other-agent", 1)],
    )
    .expect_err("duplicate ID");
    assert!(matches!(
        error,
        GatewayResolutionError::DuplicateAgentId { .. }
    ));
}

#[test]
fn public_or_uncovered_routes_fail_before_identity_resolution() {
    let (uis, _) = parsed();
    let public_gateways = parse_repository_gateways(
        GATEWAYS
            .replace("exposure = \"heph_authenticated\"", "exposure = \"public\"")
            .as_bytes(),
    )
    .config
    .expect("gateway syntax");
    let error = resolve_gateway_uis(
        &uis,
        Some(&public_gateways),
        &[binding("ui-service-agent", 1)],
    )
    .expect_err("public gateway");
    assert!(matches!(
        error,
        GatewayResolutionError::InvalidManifest { .. }
    ));

    let uncovered_gateways = parse_repository_gateways(
        GATEWAYS
            .replace("path = \"/service\"", "path = \"/other\"")
            .as_bytes(),
    )
    .config
    .expect("gateway syntax");
    let error = resolve_gateway_uis(
        &uis,
        Some(&uncovered_gateways),
        &[binding("ui-service-agent", 1)],
    )
    .expect_err("uncovered route");
    assert!(matches!(
        error,
        GatewayResolutionError::InvalidManifest { .. }
    ));
}

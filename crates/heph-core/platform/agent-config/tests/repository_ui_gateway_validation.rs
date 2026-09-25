//! Cross-manifest authorization checks for repository-owned release UIs.

use agent_config::{
    parse_repository_gateways, parse_repository_uis, validate_repository_uis_against_gateways,
};

const STATIC_UI: &str = r#"
version = 1

[[uis]]
key = "docs"
scope = "project"
label = "Docs"
icon = "book"
presentation = "iframe"
route_base = "docs"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"
"#;

const PRIVATE_SERVICE_GATEWAY: &str = r#"
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

const MANAGED_PRIVATE_UI: &str = r#"
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

#[test]
fn static_ui_without_apis_does_not_require_a_gateway_manifest() {
    let ui = parse_repository_uis(STATIC_UI.as_bytes());
    let result = validate_repository_uis_against_gateways(
        ui.config.as_ref().expect("valid static UI"),
        None,
    );
    assert!(result.is_empty(), "{result:?}");
}

#[test]
fn managed_service_and_private_api_must_be_covered_by_authenticated_service() {
    let ui = parse_repository_uis(MANAGED_PRIVATE_UI.as_bytes());
    let gateways = parse_repository_gateways(PRIVATE_SERVICE_GATEWAY.as_bytes());
    let result = validate_repository_uis_against_gateways(
        ui.config.as_ref().expect("valid managed UI"),
        Some(gateways.config.as_ref().expect("valid gateway manifest")),
    );
    assert!(result.is_empty(), "{result:?}");
}

#[test]
fn missing_or_malformed_gateway_manifests_fail_closed_without_echoing_names() {
    let ui = parse_repository_uis(MANAGED_PRIVATE_UI.as_bytes());
    let ui = ui.config.as_ref().expect("valid managed UI");
    let missing = validate_repository_uis_against_gateways(ui, None);
    assert_code(&missing, "missing_repository_ui_gateways");
    assert!(!format!("{missing:?}").contains("ui-service"));

    let mut malformed = parse_repository_gateways(PRIVATE_SERVICE_GATEWAY.as_bytes())
        .config
        .expect("valid gateway manifest");
    malformed.version = 2;
    let invalid = validate_repository_uis_against_gateways(ui, Some(&malformed));
    assert_code(&invalid, "invalid_repository_ui_gateways");
    assert!(!format!("{invalid:?}").contains("ui-service"));
}

#[test]
fn api_method_and_route_matching_is_exact_and_segment_aware() {
    let ui_source = MANAGED_PRIVATE_UI.replace(
        "kind = \"managed_service\"\ngateway_name = \"ui-service\"\nroute = \"/service/ui\"\nentrypoint = \"index.html\"",
        "kind = \"static\"\nentrypoint = \"index.html\"",
    );
    let ui_source = ui_source
        .replace(
            "[[uis.apis]]\nkey = \"service-api\"\ngateway_name = \"ui-service\"\nmethod = \"GET\"\nroute = \"/service/api\"\n",
            "[[uis.apis]]\nkey = \"service-api\"\ngateway_name = \"ui-service\"\nmethod = \"GET\"\nroute = \"/service/api\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
        );
    let ui = parse_repository_uis(ui_source.as_bytes());
    let ui = ui.config.as_ref().expect("valid static API UI");

    let covered = validate_repository_uis_against_gateways(
        ui,
        Some(
            &parse_repository_gateways(PRIVATE_SERVICE_GATEWAY.as_bytes())
                .config
                .expect("valid gateway manifest"),
        ),
    );
    assert!(covered.is_empty(), "{covered:?}");

    let wrong_method_gateway =
        PRIVATE_SERVICE_GATEWAY.replace("methods = [\"GET\"]", "methods = [\"POST\"]");
    let wrong_method = parse_repository_gateways(wrong_method_gateway.as_bytes());
    let wrong_method = validate_repository_uis_against_gateways(
        ui,
        Some(
            wrong_method
                .config
                .as_ref()
                .expect("valid gateway manifest"),
        ),
    );
    assert_code(&wrong_method, "repository_ui_gateway_route_not_declared");

    let broadening_gateway =
        PRIVATE_SERVICE_GATEWAY.replace("path = \"/service\"", "path = \"/service/api/items\"");
    let broadening = parse_repository_gateways(broadening_gateway.as_bytes());
    let broadening = validate_repository_uis_against_gateways(
        ui,
        Some(broadening.config.as_ref().expect("valid gateway manifest")),
    );
    assert_code(&broadening, "repository_ui_gateway_route_not_declared");

    let sibling_ui = ui_source.replace("route = \"/service/api\"", "route = \"/service2\"");
    let sibling_ui = parse_repository_uis(sibling_ui.as_bytes());
    let sibling_ui = sibling_ui.config.as_ref().expect("valid sibling API UI");
    let sibling = validate_repository_uis_against_gateways(
        sibling_ui,
        Some(
            &parse_repository_gateways(PRIVATE_SERVICE_GATEWAY.as_bytes())
                .config
                .expect("valid gateway manifest"),
        ),
    );
    assert_code(&sibling, "repository_ui_gateway_route_not_declared");
}

#[test]
fn public_and_stateless_service_gateways_are_denied() {
    let public = PRIVATE_SERVICE_GATEWAY.replace("heph_authenticated", "public");
    let public = parse_repository_gateways(public.as_bytes());
    let ui = parse_repository_uis(MANAGED_PRIVATE_UI.as_bytes());
    let public_result = validate_repository_uis_against_gateways(
        ui.config.as_ref().expect("valid managed UI"),
        Some(public.config.as_ref().expect("valid gateway manifest")),
    );
    assert_code(&public_result, "repository_ui_gateway_not_authenticated");

    let stateless = PRIVATE_SERVICE_GATEWAY
        .replace("handler_contract = \"http.service.v1\"\n", "handler_contract = \"http.v1\"\n")
        .replace(
            "\n[gateways.service]\nloopback_port = 8080\nreadiness_path = \"/ready\"\nhealth_path = \"/health\"\n",
            "",
        );
    let stateless = parse_repository_gateways(stateless.as_bytes());
    let stateless_result = validate_repository_uis_against_gateways(
        ui.config.as_ref().expect("valid managed UI"),
        Some(stateless.config.as_ref().expect("valid stateless gateway")),
    );
    assert_code(
        &stateless_result,
        "repository_ui_managed_service_contract_required",
    );
}

#[test]
fn persisted_ui_config_is_revalidated_before_cross_manifest_checks() {
    let parsed = parse_repository_uis(STATIC_UI.as_bytes());
    let mut config = parsed.config.expect("valid static UI");
    config.version = 2;
    let result = validate_repository_uis_against_gateways(&config, None);
    assert_code(&result, "invalid_repository_ui");
    assert_eq!(result[0].path.as_deref(), Some("uis"));
}

fn assert_code(diagnostics: &[agent_config::Diagnostic], expected: &str) {
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == expected),
        "expected {expected}, got {diagnostics:?}"
    );
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic.message.len() < 160
            && diagnostic
                .path
                .as_deref()
                .is_some_and(|path| path == "uis" || path.starts_with("uis["))
    }));
}

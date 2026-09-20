//! Semantic parser coverage for the managed Cooking reference UI.

use agent_config::ui::{UiContent, validate_repository_uis_against_gateways};
use agent_config::{
    BuildArtifactKind, NetworkProfile, RepositoryGatewaysConfig, parse, parse_repository_gateways,
    parse_repository_uis,
};
use gateway_domain::{Exposure, HttpMethod};

const AGENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/cooking/cooking-reference-service-ui/agent.toml"
));
const GATEWAYS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/cooking/cooking-reference-service-ui/heph.gateways.toml"
));
const UI: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/cooking/cooking-reference-service-ui/heph.ui.toml"
));

#[test]
fn managed_reference_fixture_binds_authenticated_service_and_runtime_files() {
    assert_agent_manifest();
    let gateway_config = parse_gateway_manifest();
    assert_ui_manifest(&gateway_config);
}

fn assert_agent_manifest() {
    let parsed_agent = parse(AGENT);
    assert!(
        parsed_agent.diagnostics.is_empty(),
        "{:?}",
        parsed_agent.diagnostics
    );
    let agent = parsed_agent.config.expect("valid managed reference agent");
    assert_eq!(
        agent.agent.key.as_deref(),
        Some("cooking-reference-service-ui")
    );
    assert_eq!(agent.guest.command, "bin/reference-ui-service");
    assert!(!agent.workspace.mount);
    assert!(!agent.state_volume.enabled);
    assert!(matches!(agent.network.profile, NetworkProfile::Disabled));

    let build = agent.build.expect("version-2 build configuration");
    assert!(matches!(build.network.profile, NetworkProfile::Disabled));
    let artifacts = build
        .artifacts
        .iter()
        .map(|artifact| {
            (
                &artifact.path,
                artifact.kind,
                artifact.media_type.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    assert!(artifacts.iter().any(|(path, kind, media_type)| {
        *path == "bin/reference-ui-service"
            && *kind == BuildArtifactKind::Executable
            && *media_type == Some("text/x-python")
    }));
    assert!(artifacts.iter().any(|(path, kind, media_type)| {
        *path == "bin/index.html"
            && *kind == BuildArtifactKind::File
            && *media_type == Some("text/html")
    }));
    assert!(artifacts.iter().any(|(path, kind, media_type)| {
        *path == "bin/heph-ui-kit-v1.0.0.css"
            && *kind == BuildArtifactKind::File
            && *media_type == Some("text/css")
    }));
}

fn parse_gateway_manifest() -> RepositoryGatewaysConfig {
    let parsed_gateways = parse_repository_gateways(GATEWAYS);
    assert!(
        parsed_gateways.diagnostics.is_empty(),
        "{:?}",
        parsed_gateways.diagnostics
    );
    let gateway_config = parsed_gateways.config.expect("valid gateway manifest");
    let gateway = gateway_config.gateways.first().expect("managed UI gateway");
    assert_eq!(gateway.name, "cooking-reference-service-ui");
    assert_eq!(gateway.agent_name, "cooking-reference-service-ui");
    assert_eq!(gateway.handler_contract, "http.service.v1");
    assert_eq!(gateway.exposure, Exposure::HephAuthenticated);
    let service = gateway.service.as_ref().expect("service settings");
    assert_eq!(service.loopback_port, 8080);
    assert_eq!(service.readiness_path, "/readyz");
    assert_eq!(service.health_path, "/healthz");
    assert_eq!(gateway.routes.len(), 1);
    assert_eq!(gateway.routes[0].path, "/reference");
    assert_eq!(gateway.routes[0].methods, vec![HttpMethod::Get]);
    gateway_config
}

fn assert_ui_manifest(gateway_config: &RepositoryGatewaysConfig) {
    let parsed_ui = parse_repository_uis(UI);
    assert!(
        parsed_ui.diagnostics.is_empty(),
        "{:?}",
        parsed_ui.diagnostics
    );
    let ui = parsed_ui.config.expect("valid managed UI manifest");
    assert_eq!(ui.uis.len(), 1);
    let declaration = &ui.uis[0];
    assert_eq!(declaration.key.as_str(), "managed-reference");
    match &declaration.content {
        UiContent::ManagedService {
            gateway_name,
            route,
            entrypoint,
        } => {
            assert_eq!(gateway_name.as_str(), "cooking-reference-service-ui");
            assert_eq!(route.as_str(), "/reference");
            assert_eq!(entrypoint.as_str(), "index.html");
        }
        UiContent::Static { .. } => panic!("managed fixture parsed as static content"),
    }
    assert!(validate_repository_uis_against_gateways(&ui, Some(gateway_config)).is_empty());
}

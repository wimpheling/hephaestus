//! Parser contract test for the publishable persistent-service example.

use agent_config::{NetworkProfile, parse, parse_repository_gateways};

const AGENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../examples/cooking/cooking-service/agent.toml"
));
const GATEWAYS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../examples/cooking/cooking-service/heph.gateways.toml"
));

#[test]
fn cooking_service_release_manifests_validate_through_platform_parser() {
    let agent = parse(AGENT);
    assert!(agent.diagnostics.is_empty(), "{:?}", agent.diagnostics);
    let agent = agent.config.expect("valid service agent manifest");
    assert_eq!(agent.agent.key.as_deref(), Some("cooking-service"));
    assert_eq!(agent.guest.command, "bin/cooking-service");
    assert!(!agent.workspace.mount);
    assert!(!agent.state_volume.enabled);
    assert!(matches!(agent.network.profile, NetworkProfile::Disabled));
    assert!(matches!(
        agent.build.as_ref().map(|build| &build.network.profile),
        Some(NetworkProfile::Disabled)
    ));

    let gateways = parse_repository_gateways(GATEWAYS);
    assert!(
        gateways.diagnostics.is_empty(),
        "{:?}",
        gateways.diagnostics
    );
    let gateway = gateways
        .config
        .expect("valid service gateway manifest")
        .gateways
        .into_iter()
        .next()
        .expect("service gateway");
    assert_eq!(gateway.agent_name, "cooking-service");
    assert_eq!(gateway.handler_contract, "http.service.v1");
    let service = gateway.service.expect("service settings");
    assert_eq!(service.loopback_port, 8080);
    assert_eq!(service.readiness_path, "/readyz");
    assert_eq!(service.health_path, "/healthz");
    assert!(gateway.secret_slots.is_empty());
    assert!(gateway.mailbox_publication_slots.is_empty());
}

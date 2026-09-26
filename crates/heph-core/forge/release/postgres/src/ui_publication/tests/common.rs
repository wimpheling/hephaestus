//! Shared UI publication test fixtures.

use super::super::load::{canonical_gateway_hash, canonical_json_hash};
use agent_config::ui::gateway_resolution::ReleaseAgentBinding;
use agent_config::ui::static_resolution::StaticArtifactCandidate;
use agent_config::{parse_repository_gateways, parse_repository_uis};
use release_domain::{AgentKey, ArtifactKind, ArtifactPath, ReleaseAgentId, ReleaseArtifactId};
use uuid::Uuid;

pub(super) const UI: &str = r#"
version = 1

[[uis]]
key = "docs"
scope = "global"
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

pub(super) const GATEWAYS: &str = r#"
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

pub(super) fn candidates() -> (Vec<StaticArtifactCandidate>, Vec<ReleaseAgentBinding>) {
    (
        vec![StaticArtifactCandidate {
            path: ArtifactPath::parse("dist/index.html").expect("artifact path"),
            id: ReleaseArtifactId::from_uuid(Uuid::from_u128(1)),
            kind: ArtifactKind::File,
            media_type: String::from("text/html"),
            size_bytes: 1,
        }],
        vec![ReleaseAgentBinding {
            agent_key: AgentKey::parse("ui-service-agent").expect("agent key"),
            release_agent_id: ReleaseAgentId::from_uuid(Uuid::from_u128(2)),
        }],
    )
}

pub(super) fn captured() -> (serde_json::Value, Vec<u8>, serde_json::Value, Vec<u8>) {
    let ui = parse_repository_uis(UI.as_bytes())
        .config
        .expect("valid UI config");
    let gateways = parse_repository_gateways(GATEWAYS.as_bytes())
        .config
        .expect("valid gateway config");
    let ui_hash = canonical_json_hash(&ui).expect("UI hash");
    let gateway_hash = canonical_gateway_hash(&gateways).expect("gateway hash");
    (
        serde_json::to_value(ui).expect("UI JSON"),
        ui_hash.to_vec(),
        serde_json::to_value(gateways).expect("gateway JSON"),
        gateway_hash.to_vec(),
    )
}

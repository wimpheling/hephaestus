use crate::{
    REPOSITORY_OCI_IMAGES_VERSION, canonical_repository_gateways, parse_repository_gateways,
    parse_repository_oci_images,
};
use capability_domain::{CapabilityOperation, CapabilityResourceKind};
use gateway_domain::ServiceLogCaptureMode;
use sha2::{Digest, Sha256};

#[test]
fn parses_repository_oci_image_manifest() {
    let manifest = format!(
        r#"
version = {REPOSITORY_OCI_IMAGES_VERSION}

[[images]]
key = "typescript-tools"
display_name = "TypeScript tools"

[images.build]
dockerfile = "containers/typescript-tools.Dockerfile"
context = "."
base = {{ key = "typescript-node-ubuntu" }}
"#
    );
    let parsed = parse_repository_oci_images(manifest.as_bytes());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let images = parsed.config.expect("valid repository OCI images");
    assert_eq!(images.images.len(), 1);
    assert_eq!(
        images.images[0].build.base.key.as_deref(),
        Some("typescript-node-ubuntu")
    );
}

#[test]
fn parses_repository_gateway_manifest_and_normalizes_declarations() {
    let first = r#"
version = 1

[[gateways]]
name = "telegram"
agent_name = "telegram-handler"
handler_contract = "http.v1"
exposure = "public"
secret_slots = ["telegram_secret", "provider_token"]
parameters = { bot = "build-notifier", enabled = true }

[[gateways.mailbox_publication_slots]]
key = "update_mailbox"
purpose = "Deliver accepted provider updates to the cooking agent."

[[gateways.routes]]
path = "/telegram/updates"
methods = ["POST", "GET"]

[[gateways]]
name = "health"
agent_name = "health-handler"
handler_contract = "http.v1"
exposure = "heph_authenticated"

[[gateways.routes]]
path = "/health"
methods = ["GET"]
"#;
    let second = r#"
version = 1

[[gateways]]
name = "health"
agent_name = "health-handler"
handler_contract = "http.v1"
exposure = "heph_authenticated"

[[gateways.routes]]
path = "/health"
methods = ["GET"]

[[gateways]]
name = "telegram"
agent_name = "telegram-handler"
handler_contract = "http.v1"
exposure = "public"
secret_slots = ["provider_token", "telegram_secret"]
parameters = { bot = "build-notifier", enabled = true }

[[gateways.mailbox_publication_slots]]
key = "update_mailbox"
purpose = "Deliver accepted provider updates to the cooking agent."

[[gateways.routes]]
path = "/telegram/updates"
methods = ["GET", "POST"]
"#;
    let parsed = parse_repository_gateways(first.as_bytes());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let config = parsed.config.expect("valid gateway manifest");
    assert_eq!(config.gateways.len(), 2);
    assert_eq!(
        config.gateways[0]
            .to_declaration()
            .expect("valid declaration")
            .validate()
            .expect("normalizable declaration")
            .len(),
        32
    );
    let slot = &config.gateways[0]
        .to_declaration()
        .expect("valid declaration")
        .mailbox_publication_slots[0];
    assert_eq!(slot.resource_kind(), CapabilityResourceKind::Mailbox);
    assert_eq!(slot.required_operation(), CapabilityOperation::Publish);
    assert!(slot.required());
    let reordered = parse_repository_gateways(second.as_bytes());
    assert!(
        reordered.diagnostics.is_empty(),
        "{:?}",
        reordered.diagnostics
    );
    assert_eq!(parsed.normalized_hash, reordered.normalized_hash);

    let canonical = canonical_repository_gateways(&config);
    let reordered_config = reordered.config.expect("valid reordered gateway manifest");
    assert_eq!(canonical, canonical_repository_gateways(&reordered_config));
    let canonical_toml = toml::to_string(&canonical).expect("canonical gateway TOML");
    let canonical_hash: [u8; 32] = Sha256::digest(canonical_toml.as_bytes()).into();
    let parser_hash = parsed
        .normalized_hash
        .expect("normalized gateway hash")
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = u8::try_from((pair[0] as char).to_digit(16).expect("hex high"))
                .expect("hex high fits");
            let low = u8::try_from((pair[1] as char).to_digit(16).expect("hex low"))
                .expect("hex low fits");
            high << 4 | low
        })
        .collect::<Vec<_>>();
    assert_eq!(parser_hash, canonical_hash);
}

#[test]
fn parses_typed_service_gateway_settings_and_rejects_mismatched_contracts() {
    let service_manifest = r#"
version = 1

[[gateways]]
name = "chat"
agent_name = "chat-handler"
handler_contract = "http.service.v1"
exposure = "heph_authenticated"

[[gateways.routes]]
path = "/chat"
methods = ["GET", "POST"]

[gateways.service]
loopback_port = 8080
readiness_path = "/ready"
health_path = "/health"
"#;
    let parsed = parse_repository_gateways(service_manifest.as_bytes());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let service = parsed
        .config
        .expect("valid service manifest")
        .gateways
        .pop()
        .expect("service gateway")
        .to_declaration()
        .expect("typed service declaration");
    assert_eq!(service.handler_contract, "http.service.v1");
    let service = service.service.expect("service settings");
    assert_eq!(service.loopback_port, 8080);
    assert_eq!(service.readiness_path.as_str(), "/ready");
    assert_eq!(service.health_path.as_str(), "/health");
    assert_eq!(service.log_capture_mode, ServiceLogCaptureMode::Disabled);

    let disabled_manifest = service_manifest.replace(
        "[gateways.service]\n",
        "[gateways.service]\nlog_capture_mode = \"disabled\"\n",
    );
    let disabled = parse_repository_gateways(disabled_manifest.as_bytes());
    assert_eq!(parsed.normalized_hash, disabled.normalized_hash);

    let application_manifest = service_manifest.replace(
        "[gateways.service]\n",
        "[gateways.service]\nlog_capture_mode = \"application\"\n",
    );
    let application = parse_repository_gateways(application_manifest.as_bytes());
    assert!(
        application.diagnostics.is_empty(),
        "{:?}",
        application.diagnostics
    );
    let application_service = application
        .config
        .expect("application logging manifest")
        .gateways
        .pop()
        .expect("application logging gateway")
        .to_declaration()
        .expect("application logging declaration")
        .service
        .expect("application logging service");
    assert_eq!(
        application_service.log_capture_mode,
        ServiceLogCaptureMode::Application
    );
    assert_ne!(parsed.normalized_hash, application.normalized_hash);

    let unknown_mode_manifest = service_manifest.replace(
        "[gateways.service]\n",
        "[gateways.service]\nlog_capture_mode = \"future\"\n",
    );
    let unknown_mode = parse_repository_gateways(unknown_mode_manifest.as_bytes());
    assert!(unknown_mode.config.is_none());
    assert!(
        unknown_mode
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "invalid_toml")
    );

    let stateless_with_service = service_manifest.replace("http.service.v1", "http.v1");
    let parsed = parse_repository_gateways(stateless_with_service.as_bytes());
    assert!(parsed.config.is_none());
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "invalid_repository_gateway_declaration")
    );

    let service_without_settings = service_manifest.replace(
        "[gateways.service]\nloopback_port = 8080\nreadiness_path = \"/ready\"\nhealth_path = \"/health\"\n",
        "",
    );
    let parsed = parse_repository_gateways(service_without_settings.as_bytes());
    assert!(parsed.config.is_none());
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "invalid_repository_gateway_declaration")
    );
}

#[test]
fn rejects_ambiguous_or_secret_bearing_repository_gateway_manifests() {
    let duplicate = r#"
version = 1
[[gateways]]
name = "echo"
agent_name = "echo-handler"
handler_contract = "http.v1"
exposure = "public"
[[gateways.routes]]
path = "/echo"
methods = ["POST"]
[[gateways]]
name = "echo"
agent_name = "echo-handler"
handler_contract = "http.v1"
exposure = "public"
[[gateways.routes]]
path = "/echo-two"
methods = ["POST"]
"#;
    let parsed = parse_repository_gateways(duplicate.as_bytes());
    assert!(parsed.config.is_none());
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "duplicate_repository_gateway_name")
    );

    let secret = r#"
version = 1
[[gateways]]
name = "echo"
agent_name = "echo-handler"
handler_contract = "http.v1"
exposure = "public"
webhook_secret = "must-never-be-accepted"
[[gateways.routes]]
path = "/echo"
methods = ["POST"]
"#;
    let parsed = parse_repository_gateways(secret.as_bytes());
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
    assert!(!format!("{:?}", parsed.diagnostics).contains("must-never-be-accepted"));

    let duplicate_mailbox_slots = r#"
version = 1
[[gateways]]
name = "echo"
agent_name = "echo-handler"
handler_contract = "http.v1"
exposure = "public"
[[gateways.mailbox_publication_slots]]
key = "agent_mailbox"
purpose = "Deliver accepted requests."
[[gateways.mailbox_publication_slots]]
key = "agent_mailbox"
purpose = "Deliver retry requests."
[[gateways.routes]]
path = "/echo"
methods = ["POST"]
"#;
    let parsed = parse_repository_gateways(duplicate_mailbox_slots.as_bytes());
    assert!(parsed.config.is_none());
    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "duplicate_repository_gateway_mailbox_publication_slot"
    }));

    let broadened_mailbox_slot = r#"
version = 1
[[gateways]]
name = "echo"
agent_name = "echo-handler"
handler_contract = "http.v1"
exposure = "public"
[[gateways.mailbox_publication_slots]]
key = "agent_mailbox"
purpose = "Deliver accepted requests."
optional_operations = ["inspect"]
[[gateways.routes]]
path = "/echo"
methods = ["POST"]
"#;
    let parsed = parse_repository_gateways(broadened_mailbox_slot.as_bytes());
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
}

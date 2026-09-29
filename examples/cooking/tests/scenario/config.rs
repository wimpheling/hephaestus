// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
pub(crate) fn enabled() -> bool {
    env::var("HEPHAESTUS_APP_COOKING_E2E").as_deref() == Ok("1")
}

pub(crate) fn source_root() -> PathBuf {
    PathBuf::from(env::var("HEPHAESTUS_COOKING_SOURCE_ROOT").expect("cooking source root"))
}

pub(crate) fn agent_artifact() -> Option<Vec<u8>> {
    enabled().then(|| {
        std::fs::read(source_root().join("cooking-agent/cooking_agent.py"))
            .expect("exact cooking agent source")
    })
}

pub(crate) fn gateway_artifact() -> Option<Vec<u8>> {
    enabled().then(|| {
        std::fs::read(
            env::var("HEPHAESTUS_COOKING_GATEWAY_ARTIFACT").expect("cooking gateway artifact path"),
        )
        .expect("read built gateway")
    })
}

/// Build the client used by Cooking probes that call the joined Caddy public
/// origin. The disposable Caddy CA is trusted explicitly in TLS mode while
/// ordinary HTTP runs retain reqwest's default behavior.
pub(crate) fn caddy_gateway_client() -> reqwest::Client {
    caddy_gateway_client_inner(None)
}

pub(crate) fn caddy_gateway_client_with_timeout(timeout: Duration) -> reqwest::Client {
    caddy_gateway_client_inner(Some(timeout))
}

pub(crate) fn caddy_gateway_client_inner(timeout: Option<Duration>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    if env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1") {
        let ca_path =
            env::var("HEPHAESTUS_CADDY_TEST_CA_CERT").expect("joined Caddy TLS fixture CA path");
        let ca_pem = fs::read(&ca_path).expect("read joined Caddy TLS fixture CA");
        let certificate =
            reqwest::Certificate::from_pem(&ca_pem).expect("parse joined Caddy TLS fixture CA");
        builder = builder.add_root_certificate(certificate);
    }
    builder
        .build()
        .expect("bounded Cooking Caddy gateway client")
}

pub(crate) fn secret_slots() -> serde_json::Value {
    serde_json::json!([
        {"key":"model","purpose":"Cooking model fixture","required":true,"delivery_modes":["brokered"],"phases":["normal"],"destinations":["api.model.example"]},
        {"key":"telegram_relay","purpose":"External cooking relay","required":true,"delivery_modes":["brokered"],"phases":["normal"],"destinations":["relay.cooking.example"]}
    ])
}

pub(crate) async fn copy_blog(target: &Path) {
    let root = source_root().join("cooking-blog");
    let mut pending = vec![(root, target.to_path_buf())];
    while let Some((source, destination)) = pending.pop() {
        tokio::fs::create_dir_all(&destination)
            .await
            .expect("blog directory");
        for entry in std::fs::read_dir(source).expect("blog sources") {
            let entry = entry.expect("blog entry");
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || name == "public" || name == "__pycache__"
            {
                continue;
            }
            let destination = destination.join(name);
            if entry.file_type().expect("blog entry type").is_dir() {
                pending.push((entry.path(), destination));
            } else {
                tokio::fs::copy(entry.path(), destination)
                    .await
                    .expect("copy blog source");
            }
        }
    }
}

pub const MODEL_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000003");
pub const RELAY_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000004");
/// Rule IDs used only by the one-shot guest-crash release. They share the
/// canonical imported secret versions while remaining unambiguous in the
/// broker dispatcher.
pub const CRASH_MODEL_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000005");
pub const CRASH_RELAY_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000006");

/// Converts a preallocated binding spec into the immutable rule consumed by
/// the shared brokered TLS upstream factory.
pub(crate) fn brokered_rule_for_spec(
    spec: &super::super::cooking_adversarial_agent::BrokeredRuleSpec,
) -> BrokeredSecretRule {
    let expected_destination = match spec.slot {
        "model" => "api.model.example",
        "telegram_relay" => "relay.cooking.example",
        _ => panic!("unexpected cooking broker slot"),
    };
    assert_eq!(spec.destination, expected_destination);
    BrokeredSecretRule {
        id: BrokeredSecretRuleId::from_uuid(spec.rule_id),
        binding_id: spec.binding_id,
        instance_revision_id: spec.instance_revision_id,
        secret_version_id: spec.secret_version_id,
        destination: Some(
            ExactHttpsOrigin::parse(format!("https://{}", spec.destination))
                .expect("crash rule origin"),
        ),
        location: HttpInjectionLocation::OutboundHeaderPrefix {
            header: HeaderName::parse("authorization").expect("crash rule header"),
            prefix: String::from("Bearer "),
        },
        gateway_route_id: None,
    }
}
// The update slice adds the held/queued v1 drain pair, one post-rotation relay
// request, and one model-held active-revocation request. The relay endpoint
// must see no corresponding request for the last event.
pub(crate) const EXPECTED_COOKING_REQUESTS: usize = 10;
pub(crate) const BASE_COOKING_REQUESTS: usize = 6;
pub(crate) const MODEL_FAULT_UPDATE: u64 = 45;
pub(crate) const RELAY_FAULT_UPDATE: u64 = 46;

pub(crate) const fn expected_upstream_requests(
    crash_mode: bool,
    update_slice: bool,
    model: bool,
) -> usize {
    if crash_mode {
        // Crash updates 51..55 have six physical calls per adapter.  The
        // canonical update slice has a different ten/nine-call budget.
        6
    } else if update_slice {
        if model {
            EXPECTED_COOKING_REQUESTS
        } else {
            EXPECTED_COOKING_REQUESTS - 1
        }
    } else {
        BASE_COOKING_REQUESTS
    }
}

pub(crate) fn parameters() -> serde_json::Value {
    if enabled() {
        serde_json::json!({"model_rule_id":MODEL_RULE,"relay_rule_id":RELAY_RULE})
    } else {
        serde_json::json!({})
    }
}

// Keep the distinct credential grants and final immutable revision binding
// sequence together so fixture authority can be reviewed in one place.

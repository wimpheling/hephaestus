//! Migration-gated structural checks for durable product events.

use super::Diagnostic;
use serde::Deserialize;
use std::{collections::BTreeMap, ffi::OsStr, fs, path::Path};

const RULES: [&str; 9] = [
    "EVT-CANONICAL-ENVELOPE",
    "EVT-CONSUMER-USES-INBOX",
    "EVT-NATS-ONLY-IN-EVENT-ADAPTERS",
    "EVT-OUTBOX-PUBLISHER-ONLY",
    "EVT-REDUCER-COVERAGE",
    "EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM",
    "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY",
    "EVT-STREAM-REAUTHORIZATION",
    "EVT-TYPED-ONEOF-PAYLOAD",
];

const PAYLOAD_VARIANTS: [&str; 16] = [
    "identity_organizations_changed",
    "organization_changed",
    "project_changed",
    "repository_changed",
    "repository_ref_changed",
    "build_changed",
    "release_changed",
    "agent_instance_changed",
    "run_changed",
    "review_changed",
    "secret_metadata_changed",
    "secret_grant_changed",
    "secret_import_changed",
    "agent_secret_binding_changed",
    "artifact_changed",
    "identity_profile_changed",
];

#[derive(Deserialize)]
struct ReducerCoverage {
    schema: String,
    variants: Vec<ReducerVariant>,
}

#[derive(Deserialize)]
struct ReducerVariant {
    field: String,
    rust_projection: String,
    phoenix_reducer: String,
}

pub(super) fn validate(root: &Path, enabled_rules: &[String], diagnostics: &mut Vec<Diagnostic>) {
    let active = RULES
        .into_iter()
        .filter(|rule| enabled_rules.iter().any(|enabled| enabled == rule))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return;
    }
    validate_contract_files(root, &active, diagnostics);
    visit_rust_sources(root, &root.join("crates"), &active, diagnostics);
}

pub(super) fn audit(root: &Path) -> BTreeMap<&'static str, usize> {
    let active = RULES.to_vec();
    let mut diagnostics = Vec::new();
    validate_contract_files(root, &active, &mut diagnostics);
    visit_rust_sources(root, &root.join("crates"), &active, &mut diagnostics);
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
}

fn validate_contract_files(root: &Path, active: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let event_path = root.join("proto/hephaestus/event/v1/event.proto");
    if active
        .iter()
        .any(|rule| matches!(*rule, "EVT-CANONICAL-ENVELOPE" | "EVT-TYPED-ONEOF-PAYLOAD"))
    {
        match fs::read_to_string(&event_path) {
            Ok(source) => validate_event_contract(&source, active, diagnostics),
            Err(_) => diagnostics.push(Diagnostic::new(
                "EVT-CANONICAL-ENVELOPE",
                "canonical product-event schema is missing",
            )),
        }
    }
    if active.contains(&"EVT-REDUCER-COVERAGE") {
        let manifest_path = root.join("proto/event-reducer-coverage.toml");
        match fs::read_to_string(manifest_path) {
            Ok(source) => validate_reducer_manifest(&source, diagnostics),
            Err(_) => diagnostics.push(Diagnostic::new(
                "EVT-REDUCER-COVERAGE",
                "product-event reducer coverage manifest is missing",
            )),
        }
    }
    validate_durable_capture(root, active, diagnostics);
    validate_stream_reauthorization(root, active, diagnostics);
}

fn validate_durable_capture(root: &Path, active: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if !active.contains(&"EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY") {
        return;
    }
    let path = root.join("migrations/0010_durable_application_events.sql");
    let Ok(source) = fs::read_to_string(path) else {
        diagnostics.push(Diagnostic::new(
            "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY",
            "durable application-event migration is missing",
        ));
        return;
    };
    for required in [
        "CREATE FUNCTION append_application_event(",
        "CREATE FUNCTION capture_direct_application_event()",
        "CREATE FUNCTION capture_parented_application_event()",
        "AFTER INSERT OR UPDATE OR DELETE",
        "PERFORM append_application_event(",
        "CREATE TRIGGER application_event_product_outbox",
    ] {
        if !source.contains(required) {
            diagnostics.push(Diagnostic::new(
                "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY",
                format!("durable event capture is missing `{required}`"),
            ));
        }
    }
}

fn validate_stream_reauthorization(
    root: &Path,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !active.contains(&"EVT-STREAM-REAUTHORIZATION") {
        return;
    }
    let application_path = root.join("crates/hephaestus-app/src/application/event.rs");
    let facade = fs::read_to_string(&application_path).unwrap_or_default();
    let application = if facade.contains("control_plane_postgres::event") {
        fs::read_to_string(root.join("crates/control-plane-postgres/src/event.rs"))
            .unwrap_or(facade)
    } else {
        facade
    };
    let watch = fs::read_to_string(root.join("crates/hephaestus-app/src/rpc/event/watch.rs"))
        .unwrap_or_default();
    let authorization_calls = application
        .matches("authorize(&mut transaction, identity, scope).await?;")
        .count();
    if authorization_calls < 2
        || !watch.contains("const READ_BATCH: i64 = 1;")
        || !watch.contains("EventError::PermissionDenied")
        || !watch.contains("Delivery::Revoked")
    {
        diagnostics.push(Diagnostic::new(
            "EVT-STREAM-REAUTHORIZATION",
            "product-event watch must reauthorize each single-event read and terminate with AccessRevoked",
        ));
    }
}

fn validate_event_contract(source: &str, active: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if active.contains(&"EVT-CANONICAL-ENVELOPE") {
        for required in [
            "message ProductEvent {",
            "OpaqueId event_id",
            "Cursor cursor",
            "EventScope scope",
            "AggregateType aggregate_type",
            "OpaqueId aggregate_id",
            "uint64 aggregate_version",
            "Timestamp occurred_at",
            "EventProvenance provenance",
            "uint32 schema_version",
        ] {
            if !source.contains(required) {
                diagnostics.push(Diagnostic::new(
                    "EVT-CANONICAL-ENVELOPE",
                    format!("ProductEvent is missing canonical declaration `{required}`"),
                ));
            }
        }
        if source.contains("rpc WatchAll") || source.contains("rpc WatchGlobal") {
            diagnostics.push(Diagnostic::new(
                "EVT-CANONICAL-ENVELOPE",
                "product-event service exposes a forbidden global watch",
            ));
        }
    }

    if active.contains(&"EVT-TYPED-ONEOF-PAYLOAD") {
        let actual = payload_variant_fields(source);
        let expected = PAYLOAD_VARIANTS.into_iter().collect::<Vec<_>>();
        if actual != expected {
            diagnostics.push(Diagnostic::new(
                "EVT-TYPED-ONEOF-PAYLOAD",
                format!(
                    "ProductEvent payload variants differ from the frozen contract: expected {}, found {}",
                    expected.join(", "),
                    actual.join(", ")
                ),
            ));
        }
    }
}

fn payload_variant_fields(source: &str) -> Vec<&str> {
    let Some((_, tail)) = source.split_once("oneof payload {") else {
        return Vec::new();
    };
    let Some((body, _)) = tail.split_once("\n  }") else {
        return Vec::new();
    };
    body.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let _message_type = words.next()?;
            let field = words.next()?;
            words.next().filter(|token| *token == "=")?;
            Some(field)
        })
        .collect()
}

fn validate_reducer_manifest(source: &str, diagnostics: &mut Vec<Diagnostic>) {
    let Ok(coverage) = toml::from_str::<ReducerCoverage>(source) else {
        diagnostics.push(Diagnostic::new(
            "EVT-REDUCER-COVERAGE",
            "product-event reducer coverage manifest is invalid TOML",
        ));
        return;
    };
    if coverage.schema != "hephaestus.event.v1.ProductEvent" {
        diagnostics.push(Diagnostic::new(
            "EVT-REDUCER-COVERAGE",
            "reducer coverage manifest names a non-canonical schema",
        ));
    }
    let fields = coverage
        .variants
        .iter()
        .map(|variant| variant.field.as_str())
        .collect::<Vec<_>>();
    let expected = PAYLOAD_VARIANTS.into_iter().collect::<Vec<_>>();
    if fields != expected {
        diagnostics.push(Diagnostic::new(
            "EVT-REDUCER-COVERAGE",
            "reducer coverage manifest does not exactly match the typed payload variants",
        ));
    }
    if coverage.variants.iter().any(|variant| {
        variant.rust_projection.trim().is_empty() || variant.phoenix_reducer.trim().is_empty()
    }) {
        diagnostics.push(Diagnostic::new(
            "EVT-REDUCER-COVERAGE",
            "every payload variant needs named Rust projection and Phoenix reducer coverage",
        ));
    }
}

fn visit_rust_sources(
    root: &Path,
    directory: &Path,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.ends_with("target")
                || path.ends_with("tests")
                || path.to_string_lossy().contains("/tests/fixtures/")
            {
                continue;
            }
            visit_rust_sources(root, &path, active, diagnostics);
        } else if path.extension() == Some(OsStr::new("rs")) {
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let relative = path.strip_prefix(root).unwrap_or(&path);
            validate_source(relative, &source, active, diagnostics);
        }
    }
}

fn validate_source(path: &Path, source: &str, active: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let rendered = path.to_string_lossy();
    if rendered.starts_with("crates/hephaestus-dev/") || rendered.starts_with("crates/rpc-proto/") {
        return;
    }
    let runtime_source = source.split("#[cfg(test)]").next().unwrap_or(source);
    let event_adapter = is_event_adapter(path);
    if uses_nats(runtime_source) && !event_adapter {
        if active.contains(&"EVT-NATS-ONLY-IN-EVENT-ADAPTERS") {
            diagnostics.push(Diagnostic::new(
                "EVT-NATS-ONLY-IN-EVENT-ADAPTERS",
                format!("{} uses NATS outside an event adapter", path.display()),
            ));
        }
        if active.contains(&"EVT-OUTBOX-PUBLISHER-ONLY") && publishes(runtime_source) {
            diagnostics.push(Diagnostic::new(
                "EVT-OUTBOX-PUBLISHER-ONLY",
                format!(
                    "{} publishes directly instead of through a designated outbox publisher",
                    path.display()
                ),
            ));
        }
    }
    if consumes_product_events(runtime_source) && !event_adapter {
        validate_product_consumer(path, runtime_source, active, diagnostics);
    }
}

fn validate_product_consumer(
    path: &Path,
    source: &str,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let claim = source
        .find("inbox")
        .or_else(|| source.find("durable_claim"));
    if active.contains(&"EVT-CONSUMER-USES-INBOX") && claim.is_none() {
        diagnostics.push(Diagnostic::new(
            "EVT-CONSUMER-USES-INBOX",
            format!(
                "{} consumes product events without a durable inbox claim",
                path.display()
            ),
        ));
    }
    if active.contains(&"EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM") {
        let effect = [".send(", ".publish(", "reqwest::", "std::process::Command"]
            .iter()
            .filter_map(|needle| source.find(needle))
            .min();
        if effect.is_some_and(|effect| claim.is_none_or(|claim| claim > effect)) {
            diagnostics.push(Diagnostic::new(
                "EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM",
                format!(
                    "{} performs an external effect before its durable claim",
                    path.display()
                ),
            ));
        }
    }
}

fn uses_nats(source: &str) -> bool {
    source.contains("async_nats::") || source.contains("use async_nats")
}

fn publishes(source: &str) -> bool {
    source.contains(".publish(") || source.contains(".publish_with_headers(")
}

fn consumes_product_events(source: &str) -> bool {
    (source.contains("hephaestus.product.event.v1") || source.contains("PRODUCT_EVENT_SUBJECT"))
        && (source.contains(".subscribe(") || source.contains(".queue_subscribe("))
}

fn is_event_adapter(path: &Path) -> bool {
    let stem = path.file_stem().and_then(OsStr::to_str);
    let rendered = path.to_string_lossy();
    matches!(
        stem,
        Some("command_transport" | "event_adapter" | "nats" | "outbox")
    ) || path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("events" | "workers" | "composition")
        )
    }) || rendered == "crates/hephaestus-app/src/lib.rs"
}

#[cfg(test)]
mod tests {
    use super::{RULES, validate_event_contract, validate_reducer_manifest, validate_source};
    use crate::checks::architecture::Diagnostic;
    use std::path::Path;

    const INVALID: &str = include_str!(
        "../../../tests/fixtures/event-architecture/invalid/src/application/service.rs"
    );
    const VALID: &str =
        include_str!("../../../tests/fixtures/event-architecture/valid/src/events/outbox.rs");
    const INVALID_EVENT: &str =
        include_str!("../../../tests/fixtures/event-architecture/invalid/proto/event.proto");
    const INVALID_REDUCERS: &str = include_str!(
        "../../../tests/fixtures/event-architecture/invalid/proto/event-reducer-coverage.toml"
    );
    const VALID_EVENT: &str = include_str!("../../../../../proto/hephaestus/event/v1/event.proto");
    const VALID_REDUCERS: &str = include_str!("../../../../../proto/event-reducer-coverage.toml");

    #[test]
    fn application_nats_publication_triggers_both_boundary_rules() {
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/application/service.rs"),
            INVALID,
            &RULES,
            &mut diagnostics,
        );
        for rule in [
            "EVT-NATS-ONLY-IN-EVENT-ADAPTERS",
            "EVT-OUTBOX-PUBLISHER-ONLY",
        ] {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule),
                "fixture did not trigger {rule}"
            );
        }
    }

    #[test]
    fn designated_event_outbox_adapter_may_publish() {
        let mut diagnostics = Vec::<Diagnostic>::new();
        validate_source(
            Path::new("crates/example/src/events/outbox.rs"),
            VALID,
            &RULES,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn designated_command_transport_may_publish_internal_commands() {
        let mut diagnostics = Vec::<Diagnostic>::new();
        validate_source(
            Path::new("crates/example/src/command_transport.rs"),
            VALID,
            &RULES,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn product_consumer_requires_inbox_before_external_effect() {
        let mut diagnostics = Vec::new();
        let source = r#"
            use async_nats::Client;
            const SUBJECT: &str = "hephaestus.product.event.v1";
            async fn consume(client: Client) {
                let _messages = client.subscribe(SUBJECT).await.unwrap();
                client.publish("external.effect", "x".into()).await.unwrap();
            }
        "#;
        validate_source(
            Path::new("crates/example/src/consumer.rs"),
            source,
            &RULES,
            &mut diagnostics,
        );
        for rule in [
            "EVT-CONSUMER-USES-INBOX",
            "EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM",
        ] {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule),
                "consumer fixture did not trigger {rule}"
            );
        }
    }

    #[test]
    fn test_only_nats_fixture_does_not_change_production_architecture() {
        let mut diagnostics = Vec::new();
        let source = r#"
            fn production() {}
            #[cfg(test)]
            mod tests {
                async fn fixture(client: async_nats::Client) {
                    let _subscription = client.subscribe("hephaestus.product.event.v1").await;
                    let _published = client.publish("external.effect", "x".into()).await;
                }
            }
        "#;
        validate_source(
            Path::new("crates/example/src/application/service.rs"),
            source,
            &RULES,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn nested_production_nats_still_triggers_boundary_rules() {
        let mut diagnostics = Vec::new();
        let source = r#"
            mod nested {
                async fn consume(client: async_nats::Client) {
                    let _subscription = client.subscribe("hephaestus.product.event.v1").await;
                    let _published = client.publish("external.effect", "x".into()).await;
                }
            }
            #[cfg(test)]
            mod tests {}
        "#;
        validate_source(
            Path::new("crates/example/src/application/service.rs"),
            source,
            &RULES,
            &mut diagnostics,
        );
        for rule in [
            "EVT-NATS-ONLY-IN-EVENT-ADAPTERS",
            "EVT-OUTBOX-PUBLISHER-ONLY",
            "EVT-CONSUMER-USES-INBOX",
            "EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM",
        ] {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule),
                "nested production fixture did not trigger {rule}"
            );
        }
    }

    #[test]
    fn malformed_event_contract_and_reducer_manifest_trigger_evt_rules() {
        let mut diagnostics = Vec::new();
        validate_event_contract(INVALID_EVENT, &RULES, &mut diagnostics);
        validate_reducer_manifest(INVALID_REDUCERS, &mut diagnostics);
        for rule in [
            "EVT-CANONICAL-ENVELOPE",
            "EVT-TYPED-ONEOF-PAYLOAD",
            "EVT-REDUCER-COVERAGE",
        ] {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule),
                "fixtures did not trigger {rule}"
            );
        }
    }

    #[test]
    fn checked_in_event_contract_and_reducer_manifest_are_complete() {
        let mut diagnostics = Vec::new();
        validate_event_contract(VALID_EVENT, &RULES, &mut diagnostics);
        validate_reducer_manifest(VALID_REDUCERS, &mut diagnostics);
        assert!(diagnostics.is_empty());
    }
}

//! Focused event architecture rule tests.

use super::{
    RULES,
    contract::{validate_event_contract, validate_reducer_manifest},
    source_scan::validate_source,
};
use crate::checks::architecture::Diagnostic;
use std::path::Path;

const INVALID: &str = include_str!(
    "../../../../tests/fixtures/event-architecture/invalid/src/application/service.rs"
);
const VALID: &str =
    include_str!("../../../../tests/fixtures/event-architecture/valid/src/events/outbox.rs");
const INVALID_DESIGNATED_ITEM: &str = include_str!(
    "../../../../tests/fixtures/event-architecture/invalid/src/events/designated_wrong_item.rs"
);
const COMMAND_VALID: &str =
    include_str!("../../../../tests/fixtures/event-architecture/valid/src/command_transport.rs");
const VALID_ATOMIC: &str = include_str!(
    "../../../../tests/fixtures/event-architecture/valid/src/application/atomic_mutation.rs"
);
const INVALID_DIRECT_WRITE: &str = include_str!(
    "../../../../tests/fixtures/event-architecture/invalid/src/application/direct_event_write.rs"
);
const INVALID_APPEND: &str = include_str!(
    "../../../../tests/fixtures/event-architecture/invalid/src/application/append_without_transaction.rs"
);
const INVALID_NESTED_WRITE: &str = include_str!(
    "../../../../tests/fixtures/event-architecture/invalid/src/application/nested_helper.rs"
);
const INVALID_EVENT: &str =
    include_str!("../../../../tests/fixtures/event-architecture/invalid/proto/event.proto");
const INVALID_REDUCERS: &str = include_str!(
    "../../../../tests/fixtures/event-architecture/invalid/proto/event-reducer-coverage.toml"
);
const VALID_EVENT: &str = include_str!("../../../../../../proto/hephaestus/event/v1/event.proto");
const VALID_REDUCERS: &str = include_str!("../../../../../../proto/event-reducer-coverage.toml");

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
        Path::new("crates/heph-app/src/event_adapter.rs"),
        VALID,
        &RULES,
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn designated_event_adapter_rejects_publication_outside_publish_pending() {
    let mut diagnostics = Vec::<Diagnostic>::new();
    validate_source(
        Path::new("crates/heph-app/src/event_adapter.rs"),
        INVALID_DESIGNATED_ITEM,
        &RULES,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "EVT-OUTBOX-PUBLISHER-ONLY"
            && diagnostic
                .message
                .contains("EventPublisher::publish_elsewhere")
    }));
}

#[test]
fn transactional_append_in_one_item_is_allowed() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/application/service.rs"),
        VALID_ATOMIC,
        &RULES,
        &mut diagnostics,
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY"),
        "valid append fixture produced {diagnostics:?}"
    );
}

#[test]
fn direct_event_table_write_is_rejected() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/application/service.rs"),
        INVALID_DIRECT_WRITE,
        &RULES,
        &mut diagnostics,
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY"
                && diagnostic.message.contains("directly writes")
        }),
        "diagnostics: {diagnostics:?}"
    );
}

#[test]
fn append_must_use_a_transaction_executor_in_the_same_item() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/application/service.rs"),
        INVALID_APPEND,
        &RULES,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY"
            && diagnostic
                .message
                .contains("recognized transaction executor")
    }));
}

#[test]
fn nested_production_helper_is_checked() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/application/service.rs"),
        INVALID_NESTED_WRITE,
        &RULES,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY"
            && diagnostic.message.contains("directly writes")
    }));
}

#[test]
fn test_only_event_table_syntax_is_ignored() {
    let mut diagnostics = Vec::new();
    let source = r#"
            fn production() {}
            #[cfg(test)]
            mod tests {
                fn fixture() {
                    let _ = sqlx::query("DELETE FROM application_events");
                }
            }
        "#;
    validate_source(
        Path::new("crates/example/src/application/service.rs"),
        source,
        &RULES,
        &mut diagnostics,
    );
    assert!(
        diagnostics.is_empty(),
        "test-only fixture produced {diagnostics:?}"
    );
}

#[test]
fn designated_command_transport_may_publish_internal_commands() {
    let mut diagnostics = Vec::<Diagnostic>::new();
    validate_source(
        Path::new("crates/example/src/command_transport.rs"),
        COMMAND_VALID,
        &RULES,
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn product_publication_is_limited_to_the_designated_item_and_path() {
    let mut diagnostics = Vec::new();
    let source = r#"
            use async_nats::Client;
            const PRODUCT_EVENT_SUBJECT: &str = "hephaestus.product.event.v1";
            async fn another_publisher(client: Client) {
                client.publish(PRODUCT_EVENT_SUBJECT, "event".into()).await.unwrap();
            }
        "#;
    validate_source(
        Path::new("crates/example/src/events/outbox.rs"),
        source,
        &RULES,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "EVT-OUTBOX-PUBLISHER-ONLY"
            && diagnostic.message.contains("designated item")
    }));
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

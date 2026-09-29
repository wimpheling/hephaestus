use super::super::source_scan::validate_source;
use super::{is_legacy_command_outbox, is_nats_event_adapter};
use crate::checks::architecture::{Diagnostic, event_architecture::RULES};
use std::path::Path;

#[test]
fn mailbox_nats_subtree_is_exactly_the_command_adapter_boundary() {
    assert!(is_nats_event_adapter(Path::new(
        "crates/heph-core/mailbox/dispatch/src/nats/handler.rs"
    )));
    assert!(is_legacy_command_outbox(Path::new(
        "crates/heph-core/mailbox/dispatch/src/nats/publisher.rs"
    )));
    assert!(is_nats_event_adapter(Path::new(
        "crates/heph-core/forge/service/src/nats.rs"
    )));
    assert!(!is_nats_event_adapter(Path::new(
        "crates/example/src/nats.rs"
    )));
    assert!(!is_nats_event_adapter(Path::new(
        "crates/example/src/nats/handler.rs"
    )));
    assert!(!is_legacy_command_outbox(Path::new(
        "crates/example/src/nats/publisher.rs"
    )));
}

const VALID_MAILBOX_HANDLER: &str = r"
    use async_nats::jetstream;
    async fn handle(message: &jetstream::Message) {
        message.double_ack().await.unwrap();
    }
";

const VALID_MAILBOX_PUBLISHER: &str = r#"
    use async_nats::jetstream;
    async fn publish(context: jetstream::Context) {
        context.publish("heph.mailbox.v1.dispatch", "command".into()).await.unwrap();
    }
"#;

const VALID_MAILBOX_TOPOLOGY: &str = r#"
    use async_nats::jetstream;
    async fn ensure(context: jetstream::Context) {
        let _ = context.get_stream("HEPH_MAILBOX_COMMANDS").await;
    }
"#;

#[test]
fn split_mailbox_nats_modules_are_valid_transport_adapters() {
    for (path, source) in [
        (
            "crates/heph-core/mailbox/dispatch/src/nats/handler.rs",
            VALID_MAILBOX_HANDLER,
        ),
        (
            "crates/heph-core/mailbox/dispatch/src/nats/publisher.rs",
            VALID_MAILBOX_PUBLISHER,
        ),
        (
            "crates/heph-core/mailbox/dispatch/src/nats/topology.rs",
            VALID_MAILBOX_TOPOLOGY,
        ),
    ] {
        let mut diagnostics = Vec::new();
        validate_source(Path::new(path), source, &RULES, &mut diagnostics);
        assert!(diagnostics.is_empty(), "{path} produced {diagnostics:?}");
    }
}

#[test]
fn generic_nats_modules_are_invalid_transport_adapters() {
    let source = r#"
        use async_nats::Client;
        async fn publish(client: Client) {
            client.publish("heph.mailbox.v1.dispatch", "command".into()).await.unwrap();
        }
    "#;
    let mut diagnostics = Vec::<Diagnostic>::new();
    validate_source(
        Path::new("crates/example/src/nats.rs"),
        source,
        &RULES,
        &mut diagnostics,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.rule_id == "EVT-NATS-ONLY-IN-EVENT-ADAPTERS" })
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "EVT-OUTBOX-PUBLISHER-ONLY")
    );
}

#[test]
fn mailbox_adapter_cannot_publish_product_events_directly() {
    let source = r#"
        use async_nats::Client;
        const PRODUCT_EVENT_SUBJECT: &str = "hephaestus.product.event.v1";
        async fn publish(client: Client) {
            client.publish(PRODUCT_EVENT_SUBJECT, "event".into()).await.unwrap();
        }
    "#;
    let mut diagnostics = Vec::<Diagnostic>::new();
    validate_source(
        Path::new("crates/heph-core/mailbox/dispatch/src/nats/publisher.rs"),
        source,
        &RULES,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "EVT-OUTBOX-PUBLISHER-ONLY"
            && diagnostic.message.contains("designated item")
    }));
}

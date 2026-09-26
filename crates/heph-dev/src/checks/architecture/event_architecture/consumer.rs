//! Event consumer, NATS, and publication boundary predicates.

use super::{
    Diagnostic,
    transaction::{rust_item_sources, without_nested_items},
};
use std::{ffi::OsStr, path::Path};

pub(super) fn validate_product_consumer(
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

pub(super) fn uses_nats(source: &str) -> bool {
    source.contains("async_nats::") || source.contains("use async_nats")
}

pub(super) fn publishes(source: &str) -> bool {
    source.contains(".publish(") || source.contains(".publish_with_headers(")
}

fn publishes_product_events(source: &str) -> bool {
    (source.contains("PRODUCT_EVENT_SUBJECT") || source.contains("hephaestus.product.event.v1"))
        && publishes(source)
}

pub(super) fn consumes_product_events(source: &str) -> bool {
    (source.contains("hephaestus.product.event.v1") || source.contains("PRODUCT_EVENT_SUBJECT"))
        && (source.contains(".subscribe(") || source.contains(".queue_subscribe("))
}

pub(super) fn is_nats_event_adapter(path: &Path) -> bool {
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
    }) || rendered == "crates/heph-app/src/lib.rs"
}

pub(super) fn is_designated_product_event_adapter(path: &Path, source: &str) -> bool {
    path == Path::new("crates/heph-app/src/event_adapter.rs")
        && rust_item_sources(source).iter().any(|item| {
            item.label == "EventPublisher::publish_pending"
                && publishes_product_events(&without_nested_items(item, source))
        })
}

pub(super) fn unauthorized_product_publication(path: &Path, source: &str) -> Option<String> {
    let designated_adapter = is_designated_product_event_adapter(path, source);
    for item in rust_item_sources(source) {
        if publishes_product_events(&without_nested_items(&item, source))
            && !(designated_adapter
                && path == Path::new("crates/heph-app/src/event_adapter.rs")
                && item.label == "EventPublisher::publish_pending")
        {
            return Some(item.label);
        }
    }
    None
}

pub(super) fn is_legacy_command_outbox(path: &Path) -> bool {
    matches!(
        path.file_stem().and_then(OsStr::to_str),
        Some("command_transport" | "outbox" | "nats")
    )
}

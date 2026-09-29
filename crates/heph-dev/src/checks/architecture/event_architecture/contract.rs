//! Event schema, reducer, migration, and stream contract checks.

use super::{Diagnostic, PAYLOAD_VARIANTS};
use serde::Deserialize;
use std::{fs, path::Path};

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

pub(super) fn validate_contract_files(
    root: &Path,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
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
    let application_path = root.join("crates/heph-app/src/application/event.rs");
    let facade = fs::read_to_string(&application_path).unwrap_or_default();
    let application = if facade.contains("control_plane_postgres::event") {
        fs::read_to_string(root.join("crates/heph-core/control-plane/postgres/src/event.rs"))
            .unwrap_or(facade)
    } else {
        facade
    };
    let watch =
        fs::read_to_string(root.join("crates/heph-app/src/rpc/event/watch.rs")).unwrap_or_default();
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

pub(super) fn validate_event_contract(
    source: &str,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
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

pub(super) fn validate_reducer_manifest(source: &str, diagnostics: &mut Vec<Diagnostic>) {
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

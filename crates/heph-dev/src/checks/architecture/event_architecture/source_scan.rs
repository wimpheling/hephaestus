//! Rust source walking and event mutation checks.

use super::{
    Diagnostic,
    consumer::{
        consumes_product_events, is_designated_product_event_adapter, is_legacy_command_outbox,
        is_nats_event_adapter, publishes, unauthorized_product_publication, uses_nats,
        validate_product_consumer,
    },
    transaction::{
        contains_direct_event_table_write, contains_transactional_append_call, rust_item_sources,
        without_nested_items,
    },
};
use std::{ffi::OsStr, fs, path::Path};

pub(super) fn visit_rust_sources(
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

pub(super) fn validate_source(
    path: &Path,
    source: &str,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let rendered = path.to_string_lossy();
    if rendered.starts_with("crates/heph-dev/")
        || rendered.starts_with("crates/heph-core/platform/rpc-proto/")
    {
        return;
    }
    // The source-to-sink checks below intentionally inspect only the production
    // prefix.  `#[cfg(test)]` modules are compiled out of production and may
    // use a broker or fixture tables freely; this checker does not attempt to
    // prove that arbitrary cfg expressions are equivalent to `cfg(test)`.
    let runtime_source = source.split("#[cfg(test)]").next().unwrap_or(source);
    if active.contains(&"EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY") {
        validate_durable_capture_source(path, runtime_source, diagnostics);
    }
    let event_adapter = is_nats_event_adapter(path);
    let designated_product_adapter = is_designated_product_event_adapter(path, runtime_source);
    let legacy_command_outbox = is_legacy_command_outbox(path);
    if uses_nats(runtime_source) && !event_adapter {
        if active.contains(&"EVT-NATS-ONLY-IN-EVENT-ADAPTERS") {
            diagnostics.push(Diagnostic::new(
                "EVT-NATS-ONLY-IN-EVENT-ADAPTERS",
                format!("{} uses NATS outside an event adapter", path.display()),
            ));
        }
        if active.contains(&"EVT-OUTBOX-PUBLISHER-ONLY")
            && publishes(runtime_source)
            && !legacy_command_outbox
            && !designated_product_adapter
        {
            diagnostics.push(Diagnostic::new(
                "EVT-OUTBOX-PUBLISHER-ONLY",
                format!(
                    "{} publishes directly instead of through a designated outbox publisher",
                    path.display()
                ),
            ));
        }
    }
    if active.contains(&"EVT-OUTBOX-PUBLISHER-ONLY")
        && let Some(label) = unauthorized_product_publication(path, runtime_source)
    {
        diagnostics.push(Diagnostic::new(
            "EVT-OUTBOX-PUBLISHER-ONLY",
            format!(
                "{}::{label} publishes product events outside the designated item `EventPublisher::publish_pending` in crates/heph-app/src/event_adapter.rs",
                path.display(),
            ),
        ));
    }
    if consumes_product_events(runtime_source) && !event_adapter {
        validate_product_consumer(path, runtime_source, active, diagnostics);
    }
}

fn validate_durable_capture_source(path: &Path, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    // This is deliberately a syntax check, rather than a SQL parser.  It
    // covers SQLx query literals in Rust items and reports the Rust item path;
    // migration triggers and transaction semantics still require runtime
    // failure-injection coverage.  Reads of these tables are allowed.  Writes
    // are not, because only the database capture function may create or
    // transition durable product-event rows. Explicit append calls are checked
    // in their own Rust item; trigger-captured business-table writes are
    // intentionally outside this source check and remain migration-gated.
    let mut found_direct_write = false;
    for item in rust_item_sources(source) {
        let label = item.label.as_str();
        let item = without_nested_items(&item, source);
        if contains_direct_event_table_write(path, &item) {
            found_direct_write = true;
            diagnostics.push(Diagnostic::new(
                "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY",
                format!(
                    "{}::{label} directly writes application_events or product_event_outbox; mutate state and call append_application_event in the same transaction instead",
                    path.display()
                ),
            ));
        }
        if item.contains("append_application_event") && !contains_transactional_append_call(&item) {
            diagnostics.push(Diagnostic::new(
                "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY",
                format!(
                    "{}::{label} calls append_application_event without a recognized transaction executor in the same Rust item",
                    path.display(),
                ),
            ));
        }
    }
    if !found_direct_write && contains_direct_event_table_write(path, source) {
        diagnostics.push(Diagnostic::new(
            "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY",
            format!(
                "{}::<unparsed item> directly writes application_events or product_event_outbox; mutate state and call append_application_event in the same transaction instead",
                path.display()
            ),
        ));
    }
}

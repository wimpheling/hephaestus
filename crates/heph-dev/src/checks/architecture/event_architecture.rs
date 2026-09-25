//! Migration-gated structural checks for durable product events.

use super::Diagnostic;
use serde::Deserialize;
use std::{collections::BTreeMap, ffi::OsStr, fs, path::Path};
use syn::{spanned::Spanned, visit::Visit};

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

const PAYLOAD_VARIANTS: [&str; 18] = [
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
    "registry_publication_changed",
    "gateway_changed",
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

#[derive(Clone)]
struct RustItemRegion {
    start: usize,
    end: usize,
    label: String,
}

fn rust_item_sources(source: &str) -> Vec<RustItemRegion> {
    let Ok(file) = syn::parse_file(source) else {
        // A malformed or macro-heavy file still gets the conservative whole
        // source check.  Valid Rust files use item spans below so nested
        // helpers cannot satisfy a sibling item's transaction requirement.
        return vec![RustItemRegion {
            start: 0,
            end: source.len(),
            label: String::from("<file>"),
        }];
    };
    let offsets = line_offsets(source);
    let mut visitor = ItemRegionVisitor {
        offsets: &offsets,
        regions: Vec::new(),
        impl_name: None,
    };
    visitor.visit_file(&file);
    if visitor.regions.is_empty() {
        return vec![RustItemRegion {
            start: 0,
            end: source.len(),
            label: String::from("<file>"),
        }];
    }
    visitor.regions
}

struct ItemRegionVisitor<'a> {
    offsets: &'a [usize],
    regions: Vec<RustItemRegion>,
    impl_name: Option<String>,
}

impl<'ast> Visit<'ast> for ItemRegionVisitor<'_> {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let previous = self.impl_name.take();
        self.impl_name = item_impl_name(item);
        syn::visit::visit_item_impl(self, item);
        self.impl_name = previous;
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.push(item.span(), item.sig.ident.to_string());
        syn::visit::visit_item_fn(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let label = self.impl_name.as_ref().map_or_else(
            || item.sig.ident.to_string(),
            |name| format!("{name}::{}", item.sig.ident),
        );
        self.push(item.span(), label);
        syn::visit::visit_impl_item_fn(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.push(item.span(), item.sig.ident.to_string());
        syn::visit::visit_trait_item_fn(self, item);
    }
}

fn item_impl_name(item: &syn::ItemImpl) -> Option<String> {
    item.trait_
        .as_ref()
        .and_then(|(_, path, _)| path.segments.last())
        .or_else(|| match item.self_ty.as_ref() {
            syn::Type::Path(path) => path.path.segments.last(),
            _ => None,
        })
        .map(|segment| segment.ident.to_string())
}

impl ItemRegionVisitor<'_> {
    fn push(&mut self, span: proc_macro2::Span, label: String) {
        let Some(start) = span_offset(self.offsets, span.start()) else {
            return;
        };
        let Some(end) = span_offset(self.offsets, span.end()) else {
            return;
        };
        if start < end {
            self.regions.push(RustItemRegion { start, end, label });
        }
    }
}

fn line_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(index + 1);
        }
    }
    offsets
}

fn span_offset(offsets: &[usize], position: proc_macro2::LineColumn) -> Option<usize> {
    offsets
        .get(position.line.checked_sub(1)?)
        .and_then(|line| line.checked_add(position.column))
}

fn without_nested_items(item: &RustItemRegion, source: &str) -> String {
    let mut cleaned = source[item.start..item.end].to_owned();
    for nested in rust_item_sources(&source[item.start..item.end]) {
        if nested.start == 0 && nested.end == cleaned.len() {
            continue;
        }
        let start = nested.start.min(cleaned.len());
        let end = nested.end.min(cleaned.len());
        if start < end {
            cleaned.replace_range(start..end, &" ".repeat(end - start));
        }
    }
    cleaned
}

fn contains_direct_event_table_write(path: &Path, source: &str) -> bool {
    let normalized = source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let writes_application_events = [
        "insert into application_events",
        "update application_events",
        "delete from application_events",
        "truncate application_events",
        "merge into application_events",
    ]
    .iter()
    .any(|verb| normalized.contains(verb));
    if writes_application_events {
        return true;
    }

    let creates_or_deletes_outbox = [
        "insert into product_event_outbox",
        "delete from product_event_outbox",
        "truncate product_event_outbox",
        "merge into product_event_outbox",
    ]
    .iter()
    .any(|verb| normalized.contains(verb));
    if creates_or_deletes_outbox {
        return true;
    }

    // The PostgreSQL event publisher is the sole provider-owned bookkeeping
    // adapter allowed to mark an already-created outbox row published or
    // failed. It never creates event rows; creation remains behind the
    // migration trigger or append_application_event.
    normalized.contains("update product_event_outbox")
        && !path.ends_with(Path::new("crates/heph-core/event/postgres/src/lib.rs"))
}

fn contains_transactional_append_call(source: &str) -> bool {
    source
        .split(';')
        .filter(|statement| statement.contains("append_application_event"))
        .any(|statement| {
            [
                ".execute(&mut *transaction",
                ".execute(&mut **transaction",
                ".execute(&mut transaction",
                ".execute(&mut *tx",
                ".execute(&mut **tx",
                ".execute(&mut tx",
                ".fetch_one(&mut *transaction",
                ".fetch_one(&mut **transaction",
                ".fetch_one(&mut *tx",
                ".fetch_one(&mut **tx",
                ".fetch_optional(&mut *transaction",
                ".fetch_optional(&mut **transaction",
                ".fetch_optional(&mut *tx",
                ".fetch_optional(&mut **tx",
                ".fetch_all(&mut *transaction",
                ".fetch_all(&mut **transaction",
                ".fetch_all(&mut *tx",
                ".fetch_all(&mut **tx",
            ]
            .iter()
            .any(|executor| statement.contains(executor))
        })
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

fn publishes_product_events(source: &str) -> bool {
    (source.contains("PRODUCT_EVENT_SUBJECT") || source.contains("hephaestus.product.event.v1"))
        && publishes(source)
}

fn consumes_product_events(source: &str) -> bool {
    (source.contains("hephaestus.product.event.v1") || source.contains("PRODUCT_EVENT_SUBJECT"))
        && (source.contains(".subscribe(") || source.contains(".queue_subscribe("))
}

fn is_nats_event_adapter(path: &Path) -> bool {
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

fn is_designated_product_event_adapter(path: &Path, source: &str) -> bool {
    path == Path::new("crates/heph-app/src/event_adapter.rs")
        && rust_item_sources(source).iter().any(|item| {
            item.label == "EventPublisher::publish_pending"
                && publishes_product_events(&without_nested_items(item, source))
        })
}

fn unauthorized_product_publication(path: &Path, source: &str) -> Option<String> {
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

fn is_legacy_command_outbox(path: &Path) -> bool {
    matches!(
        path.file_stem().and_then(OsStr::to_str),
        Some("command_transport" | "outbox" | "nats")
    )
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
    const INVALID_DESIGNATED_ITEM: &str = include_str!(
        "../../../tests/fixtures/event-architecture/invalid/src/events/designated_wrong_item.rs"
    );
    const COMMAND_VALID: &str =
        include_str!("../../../tests/fixtures/event-architecture/valid/src/command_transport.rs");
    const VALID_ATOMIC: &str = include_str!(
        "../../../tests/fixtures/event-architecture/valid/src/application/atomic_mutation.rs"
    );
    const INVALID_DIRECT_WRITE: &str = include_str!(
        "../../../tests/fixtures/event-architecture/invalid/src/application/direct_event_write.rs"
    );
    const INVALID_APPEND: &str = include_str!(
        "../../../tests/fixtures/event-architecture/invalid/src/application/append_without_transaction.rs"
    );
    const INVALID_NESTED_WRITE: &str = include_str!(
        "../../../tests/fixtures/event-architecture/invalid/src/application/nested_helper.rs"
    );
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
}

//! Rust item regions and transactional event-write helpers.

use std::path::Path;
use syn::{spanned::Spanned, visit::Visit};

#[derive(Clone)]
pub(super) struct RustItemRegion {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) label: String,
}

pub(super) fn rust_item_sources(source: &str) -> Vec<RustItemRegion> {
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

pub(super) fn without_nested_items(item: &RustItemRegion, source: &str) -> String {
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

pub(super) fn contains_direct_event_table_write(path: &Path, source: &str) -> bool {
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

pub(super) fn contains_transactional_append_call(source: &str) -> bool {
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

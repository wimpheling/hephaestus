//! Test-only spans come from parsed items, never from marker text or filenames.

use syn::{Attribute, spanned::Spanned, visit::Visit};

#[derive(Default)]
pub(super) struct Scopes {
    parsed: bool,
    ranges: Vec<(proc_macro2::LineColumn, proc_macro2::LineColumn)>,
}

impl Scopes {
    pub(super) fn parse(source: &str) -> Self {
        let Ok(file) = syn::parse_file(source) else {
            return Self::default();
        };
        let mut scopes = Self {
            parsed: true,
            ranges: Vec::new(),
        };
        scopes.visit_file(&file);
        scopes
    }

    pub(super) const fn parsed(&self) -> bool {
        self.parsed
    }

    pub(super) fn contains(&self, line: usize, column: usize, length: usize) -> bool {
        let Some(end_column) = column.checked_add(length) else {
            return false;
        };
        self.ranges.iter().any(|(start, end)| {
            (start.line, start.column) <= (line, column)
                && (line, end_column) <= (end.line, end.column)
        })
    }

    fn record(&mut self, attributes: &[Attribute], span: proc_macro2::Span) {
        if attributes.iter().any(|attribute| {
            attribute.path().is_ident("cfg")
                && attribute
                    .parse_args::<syn::Path>()
                    .is_ok_and(|condition| condition.is_ident("test"))
        }) {
            self.ranges.push((span.start(), span.end()));
        }
    }
}

impl<'ast> Visit<'ast> for Scopes {
    fn visit_variant(&mut self, variant: &'ast syn::Variant) {
        self.record(&variant.attrs, variant.span());
        syn::visit::visit_variant(self, variant);
    }

    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attributes = match item {
            syn::Item::Const(item) => &item.attrs,
            syn::Item::Enum(item) => &item.attrs,
            syn::Item::ExternCrate(item) => &item.attrs,
            syn::Item::Fn(item) => &item.attrs,
            syn::Item::ForeignMod(item) => &item.attrs,
            syn::Item::Impl(item) => &item.attrs,
            syn::Item::Macro(item) => &item.attrs,
            syn::Item::Mod(item) => &item.attrs,
            syn::Item::Static(item) => &item.attrs,
            syn::Item::Struct(item) => &item.attrs,
            syn::Item::Trait(item) => &item.attrs,
            syn::Item::TraitAlias(item) => &item.attrs,
            syn::Item::Type(item) => &item.attrs,
            syn::Item::Union(item) => &item.attrs,
            syn::Item::Use(item) => &item.attrs,
            _ => return,
        };
        self.record(attributes, item.span());
        syn::visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let attributes = match item {
            syn::ImplItem::Const(item) => &item.attrs,
            syn::ImplItem::Fn(item) => &item.attrs,
            syn::ImplItem::Type(item) => &item.attrs,
            syn::ImplItem::Macro(item) => &item.attrs,
            _ => return,
        };
        self.record(attributes, item.span());
        syn::visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        let attributes = match item {
            syn::TraitItem::Const(item) => &item.attrs,
            syn::TraitItem::Fn(item) => &item.attrs,
            syn::TraitItem::Type(item) => &item.attrs,
            syn::TraitItem::Macro(item) => &item.attrs,
            _ => return,
        };
        self.record(attributes, item.span());
        syn::visit::visit_trait_item(self, item);
    }
}

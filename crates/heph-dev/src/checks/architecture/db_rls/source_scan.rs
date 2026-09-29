//! Source traversal for the known application-role `PostgreSQL` adapters.

use super::{
    CargoMetadata, Diagnostic, RULE,
    inventory::{
        PRODUCTION_BINDINGS, PRODUCTION_POOL_FIELDS, PRODUCTION_SOURCES, PoolBinding,
        validate_pool_inventory,
    },
    scanner::FunctionScanner,
};
use std::{collections::BTreeMap, fs, path::Path};
use syn::{File, ItemFn, ItemImpl, visit::Visit};

pub(super) fn validate(
    root: &Path,
    enabled_rules: &[String],
    _metadata: &CargoMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if enabled_rules.iter().any(|rule| rule == RULE) {
        scan_sources(root, &PRODUCTION_BINDINGS, diagnostics);
    }
}

pub(super) fn audit(root: &Path) -> BTreeMap<&'static str, usize> {
    let mut diagnostics = Vec::new();
    scan_sources(root, &PRODUCTION_BINDINGS, &mut diagnostics);
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
}

fn scan_sources(root: &Path, bindings: &[PoolBinding], diagnostics: &mut Vec<Diagnostic>) {
    for source_name in PRODUCTION_SOURCES {
        let relative = Path::new(source_name);
        let path = root.join(relative);
        let Ok(source) = fs::read_to_string(&path) else {
            diagnostics.push(Diagnostic::new(
                RULE,
                format!("application-role source is missing: {}", relative.display()),
            ));
            continue;
        };
        let Ok(file) = syn::parse_file(&source) else {
            diagnostics.push(Diagnostic::new(
                RULE,
                format!(
                    "application-role source is not valid Rust: {}",
                    relative.display()
                ),
            ));
            continue;
        };
        let bindings = bindings
            .iter()
            .filter(|binding| binding.source == source_name)
            .copied()
            .collect::<Vec<_>>();
        validate_pool_inventory(
            &file,
            relative,
            &PRODUCTION_POOL_FIELDS,
            &bindings,
            diagnostics,
        );
        validate_file(&file, relative, &path, &bindings, root, diagnostics);
    }
}

pub(super) fn validate_file(
    file: &File,
    relative: &Path,
    source_path: &Path,
    bindings: &[PoolBinding],
    repository_root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut visitor = SourceVisitor {
        relative,
        source_path,
        bindings,
        repository_root,
        diagnostics,
        impl_owner: None,
    };
    visitor.visit_file(file);
}

struct SourceVisitor<'a> {
    relative: &'a Path,
    source_path: &'a Path,
    bindings: &'a [PoolBinding],
    repository_root: &'a Path,
    diagnostics: &'a mut Vec<Diagnostic>,
    impl_owner: Option<String>,
}

impl<'ast> Visit<'ast> for SourceVisitor<'_> {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        let item_name = item.sig.ident.to_string();
        let mut scanner = FunctionScanner::new(
            self.relative,
            self.source_path,
            self.bindings,
            self.repository_root,
            self.impl_owner.as_deref(),
            &item_name,
            self.diagnostics,
        );
        scanner.visit_block(&item.block);
    }

    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        let Some(owner) = type_last_ident(&item.self_ty) else {
            return;
        };
        let previous = self.impl_owner.replace(owner);
        for impl_item in &item.items {
            if let syn::ImplItem::Fn(function) = impl_item {
                let item_name = function.sig.ident.to_string();
                let mut scanner = FunctionScanner::new(
                    self.relative,
                    self.source_path,
                    self.bindings,
                    self.repository_root,
                    self.impl_owner.as_deref(),
                    &item_name,
                    self.diagnostics,
                );
                scanner.visit_block(&function.block);
            }
        }
        self.impl_owner = previous;
    }
}

fn type_last_ident(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

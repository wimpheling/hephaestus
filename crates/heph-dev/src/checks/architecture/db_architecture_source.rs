#[cfg(test)]
use super::PAGINATION_RULE;
use super::{
    ArchitectureException, Diagnostic, MIGRATION_RULE, PaginationRegistry, QueryKind, SqlVisitor,
    contains_schema_sql, impl_type_name, query_kind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    path::Path,
};
use syn::{File, ItemUse, UseTree, visit::Visit};

pub(super) fn visit_sources(
    root: &Path,
    directory: &Path,
    active: &BTreeSet<&str>,
    exceptions: &[&ArchitectureException],
    diagnostics: &mut Vec<Diagnostic>,
    pagination: &mut PaginationRegistry,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if path.is_dir() {
            if should_skip_directory(relative) {
                continue;
            }
            visit_sources(root, &path, active, exceptions, diagnostics, pagination);
        } else if path.extension() == Some(OsStr::new("rs")) {
            validate_rust_source_with_registry(
                root,
                relative,
                &path,
                active,
                exceptions,
                diagnostics,
                pagination,
            );
        } else if path.extension() == Some(OsStr::new("sql"))
            && active.contains(MIGRATION_RULE)
            && !relative.starts_with("migrations")
            && fs::read_to_string(&path).is_ok_and(|source| contains_schema_sql(&source))
        {
            diagnostics.push(Diagnostic::new(
                MIGRATION_RULE,
                format!(
                    "schema-changing SQL file is outside the root migrations boundary: {}",
                    relative.display()
                ),
            ));
        }
    }
}

fn should_skip_directory(relative: &Path) -> bool {
    relative == Path::new("target")
        || relative.starts_with(".git")
        || relative.starts_with(".local")
        || relative.starts_with("web/deps")
        || relative.starts_with("web/_build")
        || relative.starts_with("crates/heph-dev/tests/fixtures")
}

#[cfg(test)]
pub(super) fn validate_rust_source(
    root: &Path,
    relative: &Path,
    path: &Path,
    active: &BTreeSet<&str>,
    exceptions: &[&ArchitectureException],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut pagination = PaginationRegistry::default();
    validate_rust_source_with_registry(
        root,
        relative,
        path,
        active,
        exceptions,
        diagnostics,
        &mut pagination,
    );
    if active.contains(PAGINATION_RULE) {
        pagination.validate_stale(diagnostics);
    }
}

fn validate_rust_source_with_registry(
    root: &Path,
    relative: &Path,
    path: &Path,
    active: &BTreeSet<&str>,
    exceptions: &[&ArchitectureException],
    diagnostics: &mut Vec<Diagnostic>,
    pagination_registry: &mut PaginationRegistry,
) {
    let Ok(source) = fs::read_to_string(path) else {
        return;
    };
    let Ok(file) = syn::parse_file(&source) else {
        return;
    };
    let package_root = path
        .ancestors()
        .find(|ancestor| ancestor.join("Cargo.toml").is_file())
        .or_else(|| path.parent())
        .unwrap_or(path);
    let pagination = pagination_registry.contracts_for(package_root, diagnostics);
    let imports = SqlImports::collect(&file);
    let mut visitor = SqlVisitor {
        repository_root: root,
        package_root,
        source_path: path,
        path: relative,
        active,
        exceptions,
        diagnostics,
        imports,
        current_item: None,
        module_path: Vec::new(),
        function_path: Vec::new(),
        impl_type: None,
        pagination,
        query_counts: BTreeMap::new(),
        seen_queries: BTreeSet::new(),
        has_pagination_parameter: false,
    };
    visitor.visit_file(&file);
    pagination_registry.record_queries(package_root, visitor.seen_queries);
}

#[derive(Default)]
pub(super) struct SqlImports {
    pub(super) query_functions: BTreeMap<String, QueryKind>,
    pub(super) query_builders: BTreeSet<String>,
}

impl SqlImports {
    pub(super) fn collect(file: &File) -> Self {
        let mut collector = ImportCollector::default();
        collector.visit_file(file);
        collector.imports
    }
}

#[derive(Default)]
struct ImportCollector {
    imports: SqlImports,
}

impl Visit<'_> for ImportCollector {
    fn visit_item_use(&mut self, item: &ItemUse) {
        collect_use_tree(&item.tree, false, &mut self.imports);
    }
}

fn collect_use_tree(tree: &UseTree, inside_sqlx: bool, imports: &mut SqlImports) {
    match tree {
        UseTree::Path(path) => {
            collect_use_tree(&path.tree, inside_sqlx || path.ident == "sqlx", imports);
        }
        UseTree::Name(name) if inside_sqlx => {
            register_import(&name.ident.to_string(), None, imports);
        }
        UseTree::Rename(rename) if inside_sqlx => register_import(
            &rename.ident.to_string(),
            Some(rename.rename.to_string()),
            imports,
        ),
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, inside_sqlx, imports);
            }
        }
        UseTree::Glob(_) | UseTree::Name(_) | UseTree::Rename(_) => {}
    }
}

fn register_import(original: &str, rename: Option<String>, imports: &mut SqlImports) {
    let local = rename.unwrap_or_else(|| original.to_owned());
    if let Some(kind) = query_kind(original) {
        imports.query_functions.insert(local, kind);
    } else if original == "QueryBuilder" {
        imports.query_builders.insert(local);
    }
}

#[derive(Default)]
pub(super) struct RustItemIdentities {
    module_path: Vec<String>,
    function_path: Vec<String>,
    impl_type: Option<String>,
    pub(super) items: BTreeMap<String, usize>,
}

impl RustItemIdentities {
    pub(super) fn collect(file: &File) -> Self {
        let mut identities = Self::default();
        identities.visit_file(file);
        identities
    }

    fn insert(&mut self, name: String) {
        *self.items.entry(name).or_default() += 1;
    }
}

impl<'ast> Visit<'ast> for RustItemIdentities {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let Some((_, items)) = &item.content else {
            return;
        };
        self.module_path.push(item.ident.to_string());
        for item in items {
            self.visit_item(item);
        }
        self.module_path.pop();
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let mut path = self.module_path.clone();
        path.extend(self.function_path.iter().cloned());
        path.push(item.sig.ident.to_string());
        self.insert(path.join("::"));

        let previous = self.function_path.clone();
        self.function_path.push(item.sig.ident.to_string());
        syn::visit::visit_item_fn(self, item);
        self.function_path = previous;
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let previous = self.impl_type.take();
        self.impl_type = impl_type_name(&item.self_ty);
        syn::visit::visit_item_impl(self, item);
        self.impl_type = previous;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let Some(impl_type) = self.impl_type.clone() else {
            return;
        };
        let mut path = self.module_path.clone();
        path.extend(self.function_path.iter().cloned());
        path.push(impl_type.clone());
        path.push(item.sig.ident.to_string());
        self.insert(path.join("::"));

        let previous = self.function_path.clone();
        self.function_path.push(impl_type);
        self.function_path.push(item.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, item);
        self.function_path = previous;
    }
}

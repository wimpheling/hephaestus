//! Migration-gated `PostgreSQL` capability and SQL ownership checks.

use super::{ArchitectureException, CargoMetadata, CargoPackage, Diagnostic};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Expr, ExprCall, ExprMacro, File, ItemUse, Lit, UseTree, punctuated::Punctuated,
    spanned::Spanned, token::Comma, visit::Visit,
};

const SQLX_RULE: &str = "DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS";
const MIGRATION_RULE: &str = "DB-MIGRATIONS-ONLY-IN-MIGRATIONS";
const STATIC_RULE: &str = "DB-STATIC-SQL";
const PAGINATION_RULE: &str = "DB-PAGINATION-STABLE-ORDER";
const RULES: [&str; 4] = [SQLX_RULE, MIGRATION_RULE, STATIC_RULE, PAGINATION_RULE];

#[derive(Debug, serde::Deserialize)]
struct PaginationFile {
    #[serde(default)]
    queries: Vec<PaginationContract>,
}

#[derive(Debug, serde::Deserialize)]
struct PaginationContract {
    item: String,
    query_index: usize,
    order: Vec<String>,
    cursor_keys: Vec<String>,
    cursor_operator: String,
    unique_tie_breaker: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OrderKey {
    key: String,
    direction: String,
}

#[derive(Default)]
struct PaginationContracts {
    queries: BTreeMap<(String, usize), PaginationContract>,
}

impl PaginationContracts {
    fn load(package_root: &Path, diagnostics: &mut Vec<Diagnostic>) -> Self {
        let path = package_root.join("pagination.toml");
        let Ok(source) = fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(file) = toml::from_str::<PaginationFile>(&source) else {
            diagnostics.push(Diagnostic::new(
                PAGINATION_RULE,
                format!(
                    "pagination declaration {} is not valid TOML",
                    path.display()
                ),
            ));
            return Self::default();
        };
        let mut queries = BTreeMap::new();
        for contract in file.queries {
            let key = (contract.item.clone(), contract.query_index);
            if contract.query_index == 0 {
                diagnostics.push(Diagnostic::new(
                    PAGINATION_RULE,
                    format!(
                        "pagination declaration {}#{} has query_index 0; use a 1-based SQL query index",
                        path.display(), contract.item
                    ),
                ));
            }
            if queries.insert(key, contract).is_some() {
                diagnostics.push(Diagnostic::new(
                    PAGINATION_RULE,
                    format!(
                        "pagination declaration {} has a duplicate item/query_index pair",
                        path.display()
                    ),
                ));
            }
        }
        Self { queries }
    }

    fn get(&self, item: Option<&str>, query_index: usize) -> Option<&PaginationContract> {
        item.and_then(|item| self.queries.get(&(item.to_owned(), query_index)))
    }
}

/// Validates a `DB-STATIC-SQL` item selector against Rust's parsed item tree.
///
/// The general architecture exception parser validates file and line shape;
/// this rule additionally requires a unique canonical Rust item path.  That
/// prevents a short selector from accidentally covering same-named items in
/// separate modules or types.
pub(super) fn validate_exception_scope(root: &Path, scope: &str) -> Result<(), &'static str> {
    let Some((path, selector)) = scope.split_once('#') else {
        return Ok(());
    };
    if selector.trim().is_empty() {
        return Err("the item selector is empty");
    }
    let source = fs::read_to_string(root.join(path)).map_err(|_| "scoped file is not readable")?;
    let file = syn::parse_file(&source).map_err(|_| "scoped file is not valid Rust")?;
    let identities = RustItemIdentities::collect(&file);
    if identities.items.get(selector) == Some(&1) {
        Ok(())
    } else {
        Err("item selector is not an exact Rust item path")
    }
}

pub(super) fn validate(
    root: &Path,
    enabled_rules: &[String],
    metadata: &CargoMetadata,
    exceptions: &[&ArchitectureException],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let active = RULES
        .into_iter()
        .filter(|rule| enabled_rules.iter().any(|enabled| enabled == rule))
        .collect::<BTreeSet<_>>();
    if active.is_empty() {
        return;
    }
    validate_metadata(metadata, &active, diagnostics);
    visit_sources(root, root, &active, exceptions, diagnostics);
}

pub(super) fn audit(
    root: &Path,
    metadata: &CargoMetadata,
    exceptions: &[&ArchitectureException],
) -> BTreeMap<&'static str, usize> {
    let active = RULES.into_iter().collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    validate_metadata(metadata, &active, &mut diagnostics);
    visit_sources(root, root, &active, exceptions, &mut diagnostics);
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
}

fn validate_metadata(
    metadata: &CargoMetadata,
    active: &BTreeSet<&str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !active.contains(SQLX_RULE) {
        return;
    }
    let workspace = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .collect::<Vec<_>>();
    let packages_by_root = workspace
        .iter()
        .map(|package| (manifest_root(package), *package))
        .collect::<BTreeMap<_, _>>();

    for package in &workspace {
        let declaration = adapter_declaration(package, diagnostics);
        if declaration {
            continue;
        }
        if has_dev_sqlx(package) && !has_test_only_sqlx(package) {
            diagnostics.push(Diagnostic::new(
                SQLX_RULE,
                format!(
                    "workspace package {} declares a dev-only SQLx test harness without `hephaestus.sqlx_test_dependency = true`",
                    package.name
                ),
            ));
        }
        let mut visited = BTreeSet::new();
        if let Some(path) = sqlx_path(package, &packages_by_root, &mut visited) {
            diagnostics.push(Diagnostic::new(
                SQLX_RULE,
                format!(
                    "workspace package {} reaches SQLx outside a declared PostgreSQL adapter: {}",
                    package.name,
                    path.join(" -> ")
                ),
            ));
        }
    }
}

fn adapter_declaration(package: &CargoPackage, diagnostics: &mut Vec<Diagnostic>) -> bool {
    let Some(hephaestus) = package.metadata.get("hephaestus") else {
        return false;
    };
    let Some(adapter) = hephaestus.get("postgres_adapter") else {
        return false;
    };
    if adapter != true {
        diagnostics.push(Diagnostic::new(
            SQLX_RULE,
            format!(
                "workspace package {} has non-boolean or false `hephaestus.postgres_adapter`; omit it or set it to true",
                package.name
            ),
        ));
        return false;
    }
    if !package.name.ends_with("-postgres") {
        diagnostics.push(Diagnostic::new(
            SQLX_RULE,
            format!(
                "PostgreSQL adapter {} must use a package name ending in `-postgres` when `hephaestus.postgres_adapter = true`",
                package.name
            ),
        ));
    }
    let valid_context = has_valid_database_context(package);
    if !valid_context {
        diagnostics.push(Diagnostic::new(
            SQLX_RULE,
            format!(
                "PostgreSQL adapter {} requires a non-empty lowercase `hephaestus.database_context`",
                package.name
            ),
        ));
    }
    package.name.ends_with("-postgres") && valid_context
}

fn manifest_root(package: &CargoPackage) -> PathBuf {
    package
        .manifest_path
        .parent()
        .unwrap_or(&package.manifest_path)
        .to_path_buf()
}

fn sqlx_path(
    package: &CargoPackage,
    packages_by_root: &BTreeMap<PathBuf, &CargoPackage>,
    visited: &mut BTreeSet<String>,
) -> Option<Vec<String>> {
    if !visited.insert(package.id.clone()) {
        return None;
    }
    for dependency in &package.dependencies {
        if dependency.name == "sqlx" && dependency.kind.as_deref() != Some("dev") {
            return Some(vec![package.name.clone(), String::from("sqlx")]);
        }
        if dependency.kind.as_deref() == Some("dev") {
            continue;
        }
        let Some(path) = dependency.path.as_ref() else {
            continue;
        };
        let Some(target) = packages_by_root.get(path) else {
            continue;
        };
        if is_declared_adapter(target) {
            continue;
        }
        if let Some(mut path) = sqlx_path(target, packages_by_root, visited) {
            path.insert(0, package.name.clone());
            return Some(path);
        }
    }
    None
}

/// Allows `SQLx` only for an explicitly declared test harness dependency.
fn has_test_only_sqlx(package: &CargoPackage) -> bool {
    let declared = package
        .metadata
        .get("hephaestus")
        .and_then(|metadata| metadata.get("sqlx_test_dependency"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    declared
        && package.dependencies.iter().any(|dependency| {
            dependency.name == "sqlx" && dependency.kind.as_deref() == Some("dev")
        })
}

fn has_dev_sqlx(package: &CargoPackage) -> bool {
    package
        .dependencies
        .iter()
        .any(|dependency| dependency.name == "sqlx" && dependency.kind.as_deref() == Some("dev"))
}

fn is_declared_adapter(package: &CargoPackage) -> bool {
    package.name.ends_with("-postgres")
        && package
            .metadata
            .get("hephaestus")
            .and_then(|metadata| metadata.get("postgres_adapter"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        && has_valid_database_context(package)
}

fn has_valid_database_context(package: &CargoPackage) -> bool {
    package
        .metadata
        .get("hephaestus")
        .and_then(|metadata| metadata.get("database_context"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|context| {
            !context.is_empty()
                && context
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn visit_sources(
    root: &Path,
    directory: &Path,
    active: &BTreeSet<&str>,
    exceptions: &[&ArchitectureException],
    diagnostics: &mut Vec<Diagnostic>,
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
            visit_sources(root, &path, active, exceptions, diagnostics);
        } else if path.extension() == Some(OsStr::new("rs")) {
            validate_rust_source(root, relative, &path, active, exceptions, diagnostics);
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

fn validate_rust_source(
    root: &Path,
    relative: &Path,
    path: &Path,
    active: &BTreeSet<&str>,
    exceptions: &[&ArchitectureException],
    diagnostics: &mut Vec<Diagnostic>,
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
    let pagination = PaginationContracts::load(package_root, diagnostics);
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
    };
    visitor.visit_file(&file);
}

#[derive(Default)]
struct SqlImports {
    query_functions: BTreeMap<String, QueryKind>,
    query_builders: BTreeSet<String>,
}

impl SqlImports {
    fn collect(file: &File) -> Self {
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

#[derive(Clone, Copy)]
enum QueryKind {
    Inline,
    File,
    Builder,
}

#[derive(Default)]
struct RustItemIdentities {
    module_path: Vec<String>,
    function_path: Vec<String>,
    impl_type: Option<String>,
    items: BTreeMap<String, usize>,
}

impl RustItemIdentities {
    fn collect(file: &File) -> Self {
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

struct SqlVisitor<'a> {
    repository_root: &'a Path,
    package_root: &'a Path,
    source_path: &'a Path,
    path: &'a Path,
    active: &'a BTreeSet<&'a str>,
    exceptions: &'a [&'a ArchitectureException],
    diagnostics: &'a mut Vec<Diagnostic>,
    imports: SqlImports,
    current_item: Option<String>,
    module_path: Vec<String>,
    function_path: Vec<String>,
    impl_type: Option<String>,
    pagination: PaginationContracts,
    query_counts: BTreeMap<String, usize>,
}

impl SqlVisitor<'_> {
    fn validate_argument(&mut self, argument: Option<&Expr>, kind: QueryKind) {
        let Some(argument) = argument else {
            return;
        };
        let query_index = self
            .current_item
            .as_ref()
            .map(|item| {
                let count = self.query_counts.entry(item.clone()).or_default();
                *count += 1;
                *count
            })
            .unwrap_or_default();
        match argument {
            Expr::Lit(literal) => {
                if let Lit::Str(sql) = &literal.lit {
                    match kind {
                        QueryKind::Inline | QueryKind::Builder => {
                            self.validate_schema_sql(&sql.value(), None);
                            self.validate_pagination(&sql.value(), query_index);
                        }
                        QueryKind::File => {
                            self.validate_sql_file(&sql.value(), self.package_root, query_index);
                        }
                    }
                }
            }
            Expr::Macro(expression) if expression.mac.path.is_ident("include_str") => {
                if let Ok(path) = syn::parse2::<syn::LitStr>(expression.mac.tokens.clone()) {
                    let source_root = self.source_path.parent().unwrap_or(self.source_path);
                    self.validate_sql_file(&path.value(), source_root, query_index);
                }
            }
            _ if self.active.contains(STATIC_RULE) && !self.is_static_sql_exception(argument) => {
                self.diagnostics.push(Diagnostic::new(
                    STATIC_RULE,
                    format!(
                        "SQLx query in {} must receive a static string literal or include_str! source",
                        self.path.display()
                    ),
                ));
            }
            _ => {}
        }
    }

    fn is_static_sql_exception(&self, argument: &Expr) -> bool {
        let line = argument.span().start().line;
        self.exceptions.iter().any(|exception| {
            if exception.rule_id != STATIC_RULE {
                return false;
            }
            if let Some((path, item)) = exception.scope.split_once('#') {
                return Path::new(path) == self.path && self.current_item.as_deref() == Some(item);
            }
            let Some((path, line_text)) = exception.scope.rsplit_once(':') else {
                return false;
            };
            Path::new(path) == self.path && line_text.parse::<usize>().ok() == Some(line)
        })
    }

    fn validate_sql_file(&mut self, path: &str, base: &Path, query_index: usize) {
        let target = base.join(path);
        let Ok(sql) = fs::read_to_string(&target) else {
            return;
        };
        self.validate_schema_sql(&sql, Some(&target));
        self.validate_pagination(&sql, query_index);
    }

    fn validate_pagination(&mut self, sql: &str, query_index: usize) {
        let Some(contract) = self
            .pagination
            .get(self.current_item.as_deref(), query_index)
        else {
            return;
        };
        let item = contract.item.clone();
        let order = contract.order.clone();
        let cursor_keys = contract.cursor_keys.clone();
        let cursor_operator = contract.cursor_operator.clone();
        let tie_breaker = contract.unique_tie_breaker.clone();
        let fail = |message: String, diagnostics: &mut Vec<Diagnostic>| {
            diagnostics.push(Diagnostic::new(
                PAGINATION_RULE,
                format!(
                    "pagination contract {item} in {}: {message}",
                    self.path.display()
                ),
            ));
        };

        let parsed_order = match parse_order_by(sql) {
            Ok(order) => order,
            Err(message) => {
                fail(message, self.diagnostics);
                return;
            }
        };
        let declared_order = match order
            .iter()
            .map(|value| parse_order_key(value))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(order) => order,
            Err(message) => {
                fail(message, self.diagnostics);
                return;
            }
        };
        if parsed_order != declared_order {
            fail(
                format!(
                    "declared ORDER BY {declared_order:?} does not match SQL ORDER BY {parsed_order:?}"
                ),
                self.diagnostics,
            );
        }
        let order_keys = declared_order
            .iter()
            .map(|key| key.key.clone())
            .collect::<Vec<_>>();
        if cursor_keys != order_keys {
            fail(
                format!(
                    "cursor keys {cursor_keys:?} must exactly match ORDER BY keys {order_keys:?}"
                ),
                self.diagnostics,
            );
        }
        if order_keys.last() != Some(&tie_breaker) {
            fail(
                format!("unique_tie_breaker `{tie_breaker}` must be the final ORDER BY key"),
                self.diagnostics,
            );
        }
        let directions = declared_order
            .iter()
            .map(|key| key.direction.as_str())
            .collect::<BTreeSet<_>>();
        let expected_operator = if directions.len() == 1 {
            match directions.first().copied() {
                Some("ASC") => Some(">"),
                Some("DESC") => Some("<"),
                _ => None,
            }
        } else {
            None
        };
        if expected_operator != Some(cursor_operator.as_str()) {
            fail(
                format!(
                    "cursor_operator `{cursor_operator}` does not match the uniform ORDER BY direction"
                ),
                self.diagnostics,
            );
        }
        if !cursor_predicate_matches(sql, &cursor_keys, &cursor_operator) {
            fail(
                format!(
                    "SQL cursor predicate does not compare {cursor_keys:?} with `{cursor_operator}`"
                ),
                self.diagnostics,
            );
        }
    }

    fn validate_schema_sql(&mut self, sql: &str, origin: Option<&Path>) {
        let owned_by_migrations =
            origin.is_some_and(|path| path.starts_with(self.repository_root.join("migrations")));
        if self.active.contains(MIGRATION_RULE) && !owned_by_migrations && contains_schema_sql(sql)
        {
            self.diagnostics.push(Diagnostic::new(
                MIGRATION_RULE,
                format!(
                    "schema-changing SQL executes outside root migrations in {}",
                    self.path.display()
                ),
            ));
        }
    }
}

impl<'ast> Visit<'ast> for SqlVisitor<'_> {
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
        let previous_item = self.current_item.take();
        let previous_path = self.function_path.clone();
        self.function_path.push(item.sig.ident.to_string());
        self.current_item = Some(join_item_path(&self.module_path, &self.function_path));
        syn::visit::visit_item_fn(self, item);
        self.function_path = previous_path;
        self.current_item = previous_item;
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let previous = self.impl_type.take();
        self.impl_type = impl_type_name(&item.self_ty);
        syn::visit::visit_item_impl(self, item);
        self.impl_type = previous;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let previous_item = self.current_item.take();
        let previous_path = self.function_path.clone();
        if let Some(impl_type) = self.impl_type.clone() {
            self.function_path.push(impl_type);
        }
        self.function_path.push(item.sig.ident.to_string());
        self.current_item = Some(join_item_path(&self.module_path, &self.function_path));
        syn::visit::visit_impl_item_fn(self, item);
        self.function_path = previous_path;
        self.current_item = previous_item;
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Some(kind) = sqlx_query_call(call, &self.imports) {
            self.validate_argument(call.args.first(), kind);
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_macro(&mut self, expression: &'ast ExprMacro) {
        if let Some(kind) = sqlx_query_macro(expression, &self.imports)
            && let Ok(arguments) = expression
                .mac
                .parse_body_with(Punctuated::<Expr, Comma>::parse_terminated)
        {
            self.validate_argument(arguments.first(), kind);
        }
        syn::visit::visit_expr_macro(self, expression);
    }

    fn visit_file(&mut self, file: &'ast File) {
        syn::visit::visit_file(self, file);
    }
}

fn impl_type_name(self_ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(type_path) = self_ty else {
        return None;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn join_item_path(module_path: &[String], function_path: &[String]) -> String {
    module_path
        .iter()
        .chain(function_path)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("::")
}

fn sqlx_query_call(call: &ExprCall, imports: &SqlImports) -> Option<QueryKind> {
    let Expr::Path(function) = &*call.func else {
        return None;
    };
    let segments = &function.path.segments;
    let first = segments.first()?.ident.to_string();
    let last = segments.last()?.ident.to_string();
    if segments.len() == 1 {
        return imports.query_functions.get(&last).copied();
    }
    if last == "new"
        && (segments
            .iter()
            .any(|segment| segment.ident == "QueryBuilder")
            || imports.query_builders.contains(&first))
    {
        return Some(QueryKind::Builder);
    }
    (first == "sqlx").then(|| query_kind(&last)).flatten()
}

fn sqlx_query_macro(expression: &ExprMacro, imports: &SqlImports) -> Option<QueryKind> {
    let segments = &expression.mac.path.segments;
    let first = segments.first()?.ident.to_string();
    let last = segments.last()?.ident.to_string();
    if segments.len() == 1 {
        return imports.query_functions.get(&last).copied();
    }
    (first == "sqlx").then(|| query_kind(&last)).flatten()
}

fn query_kind(name: &str) -> Option<QueryKind> {
    match name {
        "query" | "query_as" | "query_scalar" => Some(QueryKind::Inline),
        "query_file" | "query_file_as" => Some(QueryKind::File),
        _ => None,
    }
}

fn parse_order_key(value: &str) -> Result<OrderKey, String> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [key, direction] if matches!(direction.to_ascii_uppercase().as_str(), "ASC" | "DESC") => {
            Ok(OrderKey {
                key: key.to_ascii_lowercase(),
                direction: direction.to_ascii_uppercase(),
            })
        }
        [key] => Ok(OrderKey {
            key: key.to_ascii_lowercase(),
            direction: String::from("ASC"),
        }),
        _ => Err(format!(
            "ORDER BY declaration `{value}` must contain one key and optional ASC/DESC"
        )),
    }
}

fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .replace(" ,", ",")
        .replace(", ", ",")
        .replace("( ", "(")
        .replace(" )", ")")
}

fn parse_order_by(sql: &str) -> Result<Vec<OrderKey>, String> {
    let normalized = normalize_sql(sql);
    let Some(start) = normalized.rfind("order by ") else {
        return Err(String::from("SQL query has no ORDER BY clause"));
    };
    let order = &normalized[start + "order by ".len()..];
    let end = [" limit ", " offset ", " fetch ", ";"]
        .iter()
        .filter_map(|marker| order.find(marker))
        .min()
        .unwrap_or(order.len());
    let order = &order[..end];
    let values = order
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_order_key)
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() {
        return Err(String::from("SQL ORDER BY clause has no keys"));
    }
    Ok(values)
}

fn cursor_predicate_matches(sql: &str, keys: &[String], operator: &str) -> bool {
    let normalized = normalize_sql(sql);
    let tuple = keys.join(",");
    let tuple_pattern = format!("({tuple}) {operator} ");
    if normalized.contains(&tuple_pattern) {
        return true;
    }
    keys.len() == 1 && normalized.contains(&format!("{} {operator} ", keys[0]))
}

fn contains_schema_sql(sql: &str) -> bool {
    let words = sql
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    words.windows(2).any(|pair| {
        matches!(
            (pair[0].as_str(), pair[1].as_str()),
            (
                "CREATE" | "ALTER" | "DROP",
                "TABLE" | "INDEX" | "SCHEMA" | "TYPE" | "POLICY" | "FUNCTION" | "TRIGGER" | "ROLE"
            ) | ("TRUNCATE", "TABLE")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        PAGINATION_RULE, RULES, STATIC_RULE, audit, contains_schema_sql, validate_exception_scope,
        validate_metadata, validate_rust_source,
    };
    use crate::checks::architecture::{
        ArchitectureException, CargoDependency, CargoMetadata, CargoPackage, Diagnostic,
    };
    use serde_json::json;
    use std::{
        collections::BTreeSet,
        fs,
        path::{Path, PathBuf},
    };
    use tempfile::tempdir;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/db-architecture")
            .join(name)
    }

    fn package(
        root: &Path,
        name: &str,
        metadata: serde_json::Value,
        dependencies: Vec<CargoDependency>,
    ) -> CargoPackage {
        CargoPackage {
            id: format!("{name} 0.1.0"),
            name: name.to_owned(),
            manifest_path: root.join(name).join("Cargo.toml"),
            metadata,
            dependencies,
        }
    }

    fn path_dependency(name: &str, path: PathBuf) -> CargoDependency {
        CargoDependency {
            name: name.to_owned(),
            path: Some(path),
            kind: None,
        }
    }

    fn sqlx_dependency() -> CargoDependency {
        CargoDependency {
            name: String::from("sqlx"),
            path: None,
            kind: None,
        }
    }

    fn dev_sqlx_dependency() -> CargoDependency {
        CargoDependency {
            name: String::from("sqlx"),
            path: None,
            kind: Some(String::from("dev")),
        }
    }

    fn sqlx_metadata_diagnostics(metadata: &CargoMetadata) -> Vec<Diagnostic> {
        let active = BTreeSet::from([super::SQLX_RULE]);
        let mut diagnostics = Vec::new();
        validate_metadata(metadata, &active, &mut diagnostics);
        diagnostics
    }

    fn static_exception(scope: &str, rule_id: &str) -> ArchitectureException {
        ArchitectureException {
            rule_id: rule_id.to_owned(),
            scope: scope.to_owned(),
            rationale: String::from("fixture exception"),
            owner: String::from("architecture-test"),
            expires: Some(String::from("2099-01-01")),
            tracking_task: None,
        }
    }

    fn scan_dynamic_queries(source: &str, exceptions: &[ArchitectureException]) -> Vec<Diagnostic> {
        let root = tempdir().expect("temporary scanner root");
        let source_path = root.path().join("src/lib.rs");
        fs::create_dir_all(source_path.parent().expect("source parent"))
            .expect("create source parent");
        fs::write(&source_path, source).expect("write source");
        let active = BTreeSet::from([STATIC_RULE]);
        let exception_refs = exceptions.iter().collect::<Vec<_>>();
        let mut diagnostics = Vec::new();
        validate_rust_source(
            root.path(),
            Path::new("src/lib.rs"),
            &source_path,
            &active,
            &exception_refs,
            &mut diagnostics,
        );
        diagnostics
    }

    fn scope_result(source: &str, scope: &str) -> Result<(), &'static str> {
        let root = tempdir().expect("temporary scope root");
        let source_path = root.path().join("src/lib.rs");
        fs::create_dir_all(source_path.parent().expect("source parent"))
            .expect("create source parent");
        fs::write(&source_path, source).expect("write source");
        validate_exception_scope(root.path(), scope)
    }

    fn scan_pagination_fixture(name: &str) -> Vec<Diagnostic> {
        let root = fixture(name);
        let source_path = root.join("src/lib.rs");
        let source = fs::read_to_string(&source_path).expect("pagination fixture source");
        let active = BTreeSet::from([PAGINATION_RULE]);
        let mut diagnostics = Vec::new();
        validate_rust_source(
            &root,
            Path::new("src/lib.rs"),
            &source_path,
            &active,
            &[],
            &mut diagnostics,
        );
        assert!(!source.is_empty());
        diagnostics
    }

    #[test]
    fn explicitly_marked_dev_sqlx_harness_is_allowed() {
        let root = fixture("valid");
        let harness = package(
            &root,
            "harness",
            json!({"hephaestus": {"sqlx_test_dependency": true}}),
            vec![dev_sqlx_dependency()],
        );
        let metadata = CargoMetadata {
            workspace_members: vec![harness.id.clone()],
            packages: vec![harness],
            workspace_root: root.clone(),
        };
        assert!(audit(&root, &metadata, &[]).is_empty());
    }

    #[test]
    fn valid_adapter_is_a_capability_firewall_and_owns_only_static_queries() {
        let root = fixture("valid");
        let adapter = package(
            &root,
            "adapter-postgres",
            json!({"hephaestus": {"postgres_adapter": true, "database_context": "fixture"}}),
            vec![sqlx_dependency()],
        );
        let application = package(
            &root,
            "application",
            serde_json::Value::Null,
            vec![path_dependency(
                "adapter-postgres",
                root.join("adapter-postgres"),
            )],
        );
        let metadata = CargoMetadata {
            workspace_members: vec![adapter.id.clone(), application.id.clone()],
            packages: vec![adapter, application],
            workspace_root: root.clone(),
        };
        assert!(audit(&root, &metadata, &[]).is_empty());
    }

    #[test]
    fn valid_pagination_contract_matches_order_cursor_and_tie_breaker() {
        assert!(scan_pagination_fixture("valid/adapter-postgres").is_empty());
    }

    #[test]
    fn pagination_contract_reports_missing_order() {
        let diagnostics = scan_pagination_fixture("invalid/pagination-missing-order");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == PAGINATION_RULE
                && diagnostic.message.contains("no ORDER BY clause")
                && diagnostic.message.contains("list")
        }));
    }

    #[test]
    fn pagination_contract_reports_mismatched_cursor_keys() {
        let diagnostics = scan_pagination_fixture("invalid/pagination-mismatched-cursor");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == PAGINATION_RULE
                && diagnostic.message.contains("cursor keys")
                && diagnostic.message.contains("list")
        }));
    }

    #[test]
    fn pagination_contract_reports_missing_unique_tie_breaker() {
        let diagnostics = scan_pagination_fixture("invalid/pagination-missing-tie-breaker");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == PAGINATION_RULE
                && diagnostic.message.contains("unique_tie_breaker")
                && diagnostic.message.contains("list")
        }));
    }

    #[test]
    fn production_postgres_adapter_requires_the_postgres_package_suffix() {
        let root = fixture("invalid");
        let adapter = package(
            &root,
            "adapter",
            json!({"hephaestus": {"postgres_adapter": true, "database_context": "fixture"}}),
            vec![sqlx_dependency()],
        );
        let metadata = CargoMetadata {
            workspace_members: vec![adapter.id.clone()],
            packages: vec![adapter],
            workspace_root: root,
        };

        let diagnostics = sqlx_metadata_diagnostics(&metadata);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("must use a package name ending in `-postgres`")
        }));
    }

    #[test]
    fn postgres_suffix_without_adapter_metadata_does_not_authorize_sqlx() {
        let root = fixture("invalid");
        let suffix_package = package(
            &root,
            "suffix-without-metadata-postgres",
            serde_json::Value::Null,
            vec![sqlx_dependency()],
        );
        let consumer = package(
            &root,
            "suffix-consumer",
            serde_json::Value::Null,
            vec![path_dependency(
                "suffix-without-metadata-postgres",
                root.join("suffix-without-metadata-postgres"),
            )],
        );
        let metadata = CargoMetadata {
            workspace_members: vec![suffix_package.id.clone(), consumer.id.clone()],
            packages: vec![suffix_package, consumer],
            workspace_root: root,
        };

        let diagnostics = sqlx_metadata_diagnostics(&metadata);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains(
                "workspace package suffix-without-metadata-postgres reaches SQLx outside a declared PostgreSQL adapter",
            )
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains(
                "workspace package suffix-consumer reaches SQLx outside a declared PostgreSQL adapter",
            )
        }));
    }

    #[test]
    fn invalid_fixture_covers_direct_transitive_metadata_ddl_and_dynamic_sql() {
        let root = fixture("invalid");
        let direct = package(
            &root,
            "direct",
            serde_json::Value::Null,
            vec![sqlx_dependency()],
        );
        let consumer = package(
            &root,
            "consumer",
            serde_json::Value::Null,
            vec![path_dependency("direct", root.join("direct"))],
        );
        let invalid_adapter = package(
            &root,
            "invalid-adapter",
            json!({"hephaestus": {"postgres_adapter": true, "database_context": "fixture"}}),
            vec![sqlx_dependency()],
        );
        let metadata = CargoMetadata {
            workspace_members: vec![
                direct.id.clone(),
                consumer.id.clone(),
                invalid_adapter.id.clone(),
            ],
            packages: vec![direct, consumer, invalid_adapter],
            workspace_root: root.clone(),
        };
        let counts = audit(&root, &metadata, &[]);
        assert_eq!(counts.get(RULES[0]), Some(&4));
        assert_eq!(counts.get(RULES[1]), Some(&4));
        assert_eq!(counts.get(RULES[2]), Some(&3));
    }

    #[test]
    fn schema_classifier_is_case_and_layout_insensitive() {
        assert!(contains_schema_sql("create\n table example(id int)"));
        assert!(contains_schema_sql("ALTER TYPE status ADD VALUE 'done'"));
        assert!(!contains_schema_sql("SELECT * FROM example"));
    }

    #[test]
    fn item_exception_is_exact_and_does_not_cover_neighbor_or_other_rule() {
        let source = r"
use sqlx::query;
fn allowed(value: String) { query(&value); }
fn neighbor(value: String) { query(&value); }
";
        let diagnostics = scan_dynamic_queries(
            source,
            &[
                static_exception("src/lib.rs#allowed", STATIC_RULE),
                static_exception("src/lib.rs#neighbor", "DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS"),
                static_exception("other.rs#allowed", STATIC_RULE),
            ],
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "neighboring dynamic query must remain reported"
        );
        assert_eq!(diagnostics[0].rule_id, STATIC_RULE);
    }

    #[test]
    fn line_exception_matches_only_the_exact_query_line() {
        let source = "use sqlx::query;\nfn first(value: String) { query(&value); }\nfn second(value: String) { query(&value); }\n";
        let diagnostics =
            scan_dynamic_queries(source, &[static_exception("src/lib.rs:2", STATIC_RULE)]);
        assert_eq!(
            diagnostics.len(),
            1,
            "the query on the other line remains reported"
        );
    }

    #[test]
    fn impl_method_exception_uses_the_qualified_item_name() {
        let source = r"
use sqlx::query;
struct Database;
impl Database {
    fn create(value: String) { query(&value); }
    fn neighbor(value: String) { query(&value); }
}
";
        let diagnostics = scan_dynamic_queries(
            source,
            &[static_exception("src/lib.rs#Database::create", STATIC_RULE)],
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "the neighboring method remains reported"
        );
    }

    #[test]
    fn same_named_functions_require_module_qualified_selectors() {
        let source = r"
use sqlx::query;
mod one { pub fn allowed(value: String) { query(&value); } }
mod two { pub fn allowed(value: String) { query(&value); } }
";
        assert!(scope_result(source, "src/lib.rs#allowed").is_err());
        let diagnostics = scan_dynamic_queries(
            source,
            &[static_exception("src/lib.rs#allowed", STATIC_RULE)],
        );
        assert_eq!(diagnostics.len(), 2);
        let diagnostics = scan_dynamic_queries(
            source,
            &[static_exception("src/lib.rs#one::allowed", STATIC_RULE)],
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "module-qualified selector suppresses one item"
        );
    }

    #[test]
    fn same_named_methods_require_module_and_type_qualified_selectors() {
        let source = r"
use sqlx::query;
mod one { struct Thing; impl Thing { fn run(value: String) { query(&value); } } }
mod two { struct Thing; impl Thing { fn run(value: String) { query(&value); } } }
";
        assert!(scope_result(source, "src/lib.rs#Thing::run").is_err());
        let diagnostics = scan_dynamic_queries(
            source,
            &[static_exception("src/lib.rs#Thing::run", STATIC_RULE)],
        );
        assert_eq!(diagnostics.len(), 2);
        let diagnostics = scan_dynamic_queries(
            source,
            &[static_exception("src/lib.rs#one::Thing::run", STATIC_RULE)],
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "fully qualified method suppresses one item"
        );
    }

    #[test]
    fn outer_function_exception_does_not_cover_nested_function() {
        let source = r"
use sqlx::query;
fn outer(value: String) { query(&value); fn nested(value: String) { query(&value); } }
";
        let diagnostics =
            scan_dynamic_queries(source, &[static_exception("src/lib.rs#outer", STATIC_RULE)]);
        assert_eq!(diagnostics.len(), 1);
        assert!(scope_result(source, "src/lib.rs#outer::nested").is_ok());
    }

    #[test]
    fn nonexistent_qualified_method_is_rejected_without_cross_type_matching() {
        let source = r"
struct A;
impl A { fn run(value: String) { let _ = value; } }
struct B;
impl B { fn other(value: String) { let _ = value; } }
";
        assert!(scope_result(source, "src/lib.rs#A::other").is_err());
    }
}

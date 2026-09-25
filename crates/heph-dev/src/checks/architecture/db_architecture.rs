//! Migration-gated `PostgreSQL` capability and SQL ownership checks.

use super::{ArchitectureException, CargoMetadata, CargoPackage, Diagnostic};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::{
    Expr, ExprCall, ExprMacro, File, Lit, punctuated::Punctuated, spanned::Spanned, token::Comma,
    visit::Visit,
};

const SQLX_RULE: &str = "DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS";
const MIGRATION_RULE: &str = "DB-MIGRATIONS-ONLY-IN-MIGRATIONS";
const STATIC_RULE: &str = "DB-STATIC-SQL";
const PAGINATION_RULE: &str = "DB-PAGINATION-STABLE-ORDER";
const RULES: [&str; 4] = [SQLX_RULE, MIGRATION_RULE, STATIC_RULE, PAGINATION_RULE];

#[derive(Clone, Copy)]
pub(super) enum QueryKind {
    Inline,
    File,
    Builder,
}

#[path = "db_architecture_pagination.rs"]
mod pagination;
use pagination::{PaginationContract, PaginationContracts, PaginationRegistry};
#[path = "db_architecture_sql.rs"]
mod sql;
use sql::{
    contains_schema_sql, cursor_predicate_matches, is_paginated_sql, normalize_sql, parse_order_by,
    parse_order_key, uuid_row_lookup_matches,
};
#[path = "db_architecture_metadata.rs"]
mod metadata;
use metadata::validate_metadata;
#[path = "db_architecture_source.rs"]
mod source;
#[cfg(test)]
use source::validate_rust_source;
use source::{RustItemIdentities, SqlImports, visit_sources};

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
    let mut pagination = PaginationRegistry::default();
    visit_sources(
        root,
        root,
        &active,
        exceptions,
        diagnostics,
        &mut pagination,
    );
    if active.contains(PAGINATION_RULE) {
        pagination.validate_stale(diagnostics);
    }
}

pub(super) fn audit(
    root: &Path,
    metadata: &CargoMetadata,
    exceptions: &[&ArchitectureException],
) -> BTreeMap<&'static str, usize> {
    let active = RULES.into_iter().collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    validate_metadata(metadata, &active, &mut diagnostics);
    let mut pagination = PaginationRegistry::default();
    visit_sources(
        root,
        root,
        &active,
        exceptions,
        &mut diagnostics,
        &mut pagination,
    );
    if active.contains(PAGINATION_RULE) {
        pagination.validate_stale(&mut diagnostics);
    }
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
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
    seen_queries: BTreeSet<(String, usize)>,
    has_pagination_parameter: bool,
}

impl SqlVisitor<'_> {
    fn fail_pagination(&mut self, item: Option<&str>, query_index: usize, message: &str) {
        let item = item.unwrap_or("<unknown>");
        self.diagnostics.push(Diagnostic::new(
            PAGINATION_RULE,
            format!(
                "pagination query {item}#{query_index} in {}: {message}",
                self.path.display()
            ),
        ));
    }

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
        if let Some(item) = self.current_item.as_ref() {
            self.seen_queries.insert((item.clone(), query_index));
        }
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
        if !self.active.contains(PAGINATION_RULE) {
            return;
        }
        let item_path = self.current_item.clone();
        let Some(contract) =
            self.pagination
                .get(item_path.as_deref(), query_index)
                .map(|contract| PaginationContract {
                    item: contract.item.clone(),
                    query_index: contract.query_index,
                    order: contract.order.clone(),
                    cursor_keys: contract.cursor_keys.clone(),
                    cursor_operator: contract.cursor_operator.clone(),
                    unique_tie_breaker: contract.unique_tie_breaker.clone(),
                    unique_keys: contract.unique_keys.clone(),
                    cursor_mode: contract.cursor_mode.clone(),
                })
        else {
            if self.has_pagination_parameter && is_paginated_sql(sql) {
                self.fail_pagination(
                    item_path.as_deref(),
                    query_index,
                    "paginated SQL requires a pagination.toml declaration",
                );
            }
            return;
        };
        self.validate_declared_pagination(sql, &contract);
    }

    fn validate_declared_pagination(&mut self, sql: &str, contract: &PaginationContract) {
        let item = contract.item.clone();
        let unique_keys = &contract.unique_keys;
        let tie_breaker = &contract.unique_tie_breaker;
        let cursor_mode = &contract.cursor_mode;
        let fail = |message: String, diagnostics: &mut Vec<Diagnostic>| {
            diagnostics.push(Diagnostic::new(
                PAGINATION_RULE,
                format!(
                    "pagination contract {item} in {}: {message}",
                    self.path.display()
                ),
            ));
        };

        if cursor_mode == "stored_function" {
            if !self.inspect_run_https_uses_contract_is_intact(sql) {
                fail(
                    String::from(
                        "stored_function contract must match migration 0061 inspect_run_https_uses signature, cursor predicate, ORDER BY audit.id, and bounded LIMIT",
                    ),
                    self.diagnostics,
                );
            }
            return;
        }
        if !is_paginated_sql(sql) {
            fail(
                String::from(
                    "declared query is not canonical paginated SQL (requires LIMIT and a cursor predicate)",
                ),
                self.diagnostics,
            );
            return;
        }
        if !unique_keys.iter().any(|key| key == tie_breaker) {
            fail(
                format!("unique_tie_breaker `{tie_breaker}` has no explicit uniqueness metadata"),
                self.diagnostics,
            );
        }
        if cursor_mode == "uuid_row_lookup" && !uuid_row_lookup_matches(sql) {
            fail(
                String::from("uuid_row_lookup cursor_mode requires a cursor row lookup by UUID"),
                self.diagnostics,
            );
        }

        self.validate_order_contract(sql, contract, &fail);
    }

    fn validate_order_contract(
        &mut self,
        sql: &str,
        contract: &PaginationContract,
        fail: &dyn Fn(String, &mut Vec<Diagnostic>),
    ) {
        let order = &contract.order;
        let cursor_keys = &contract.cursor_keys;
        let cursor_operator = &contract.cursor_operator;
        let tie_breaker = &contract.unique_tie_breaker;
        let cursor_mode = &contract.cursor_mode;
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
        if *cursor_keys != order_keys {
            fail(
                format!(
                    "cursor keys {cursor_keys:?} must exactly match ORDER BY keys {order_keys:?}"
                ),
                self.diagnostics,
            );
        }
        if order_keys.last().map(String::as_str) != Some(tie_breaker) {
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
        if expected_operator != Some(cursor_operator) {
            fail(
                format!(
                    "cursor_operator `{cursor_operator}` does not match the uniform ORDER BY direction"
                ),
                self.diagnostics,
            );
        }
        if !cursor_predicate_matches(sql, cursor_keys, cursor_operator, cursor_mode) {
            fail(
                format!(
                    "SQL cursor predicate does not compare {cursor_keys:?} with `{cursor_operator}`"
                ),
                self.diagnostics,
            );
        }
    }

    fn inspect_run_https_uses_contract_is_intact(&self, sql: &str) -> bool {
        // Migration 0061 owns this function's SQL body. Keep the Rust escape
        // hatch pinned to its public signature and the stable cursor/limit
        // markers so a migration edit cannot silently invalidate the page.
        if normalize_sql(sql) != "select * from inspect_run_https_uses($1,$2,$3)" {
            return false;
        }
        let migration = self
            .repository_root
            .join("migrations/0061_run_provenance_inspection.sql");
        let Ok(source) = fs::read_to_string(migration) else {
            return false;
        };
        let normalized = normalize_sql(&source);
        normalized.contains(
            "create function inspect_run_https_uses(target_run uuid,after_id uuid,page_size integer)",
        ) && (normalized.contains("and (after_id is null or audit.id > after_id)")
            || normalized.contains("where (after_id is null or audit.id > after_id)"))
            && normalized.contains("order by audit.id")
            && normalized.contains("limit greatest(1,least(page_size,201))")
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
        let previous_pagination = self.has_pagination_parameter;
        self.function_path.push(item.sig.ident.to_string());
        self.current_item = Some(join_item_path(&self.module_path, &self.function_path));
        self.has_pagination_parameter = has_pagination_parameter(&item.sig);
        syn::visit::visit_item_fn(self, item);
        self.function_path = previous_path;
        self.current_item = previous_item;
        self.has_pagination_parameter = previous_pagination;
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
        let previous_pagination = self.has_pagination_parameter;
        if let Some(impl_type) = self.impl_type.clone() {
            self.function_path.push(impl_type);
        }
        self.function_path.push(item.sig.ident.to_string());
        self.current_item = Some(join_item_path(&self.module_path, &self.function_path));
        self.has_pagination_parameter = has_pagination_parameter(&item.sig);
        syn::visit::visit_impl_item_fn(self, item);
        self.function_path = previous_path;
        self.current_item = previous_item;
        self.has_pagination_parameter = previous_pagination;
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

pub(super) fn impl_type_name(self_ty: &syn::Type) -> Option<String> {
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

fn has_pagination_parameter(signature: &syn::Signature) -> bool {
    // Canonical page items expose `page`, `request`, `cursor`, or `before`; an
    // `after` cursor counts only when paired with a page `size`.
    // Fixed-size worker batches therefore remain outside this rule.
    let names = signature
        .inputs
        .iter()
        .filter_map(|argument| match argument {
            syn::FnArg::Typed(argument) => match &*argument.pat {
                syn::Pat::Ident(identifier) => Some(identifier.ident.to_string()),
                _ => None,
            },
            syn::FnArg::Receiver(_) => None,
        })
        .collect::<BTreeSet<_>>();
    names
        .iter()
        .any(|name| matches!(name.as_str(), "page" | "request" | "cursor" | "before"))
        || (names.contains("after") && names.contains("size"))
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

pub(super) fn query_kind(name: &str) -> Option<QueryKind> {
    match name {
        "query" | "query_as" | "query_scalar" => Some(QueryKind::Inline),
        "query_file" | "query_file_as" => Some(QueryKind::File),
        _ => None,
    }
}

#[cfg(test)]
#[path = "db_architecture_tests.rs"]
mod tests;

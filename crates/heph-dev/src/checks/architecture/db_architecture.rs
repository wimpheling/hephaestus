//! Migration-gated `PostgreSQL` capability and SQL ownership checks.

use super::{ArchitectureException, CargoMetadata, CargoPackage, Diagnostic};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::{Expr, File, Lit, spanned::Spanned};

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
#[path = "db_architecture_traversal.rs"]
mod traversal;
#[path = "db_architecture_visitor_pagination.rs"]
mod visitor_pagination;
use traversal::{impl_type_name, query_kind};

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

#[cfg(test)]
#[path = "db_architecture_tests.rs"]
mod tests;

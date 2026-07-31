//! Migration-gated `PostgreSQL` capability and SQL ownership checks.

use super::{CargoMetadata, CargoPackage, Diagnostic};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Expr, ExprCall, ExprMacro, File, ItemUse, Lit, UseTree, punctuated::Punctuated, token::Comma,
    visit::Visit,
};

const SQLX_RULE: &str = "DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS";
const MIGRATION_RULE: &str = "DB-MIGRATIONS-ONLY-IN-MIGRATIONS";
const STATIC_RULE: &str = "DB-STATIC-SQL";
const RULES: [&str; 3] = [SQLX_RULE, MIGRATION_RULE, STATIC_RULE];

pub(super) fn validate(
    root: &Path,
    enabled_rules: &[String],
    metadata: &CargoMetadata,
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
    visit_sources(root, root, &active, diagnostics);
}

pub(super) fn audit(root: &Path, metadata: &CargoMetadata) -> BTreeMap<&'static str, usize> {
    let active = RULES.into_iter().collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    validate_metadata(metadata, &active, &mut diagnostics);
    visit_sources(root, root, &active, &mut diagnostics);
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
    let valid_context = hephaestus
        .get("database_context")
        .and_then(|value| value.as_str())
        .is_some_and(|context| {
            !context.is_empty()
                && context
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        });
    if !valid_context {
        diagnostics.push(Diagnostic::new(
            SQLX_RULE,
            format!(
                "PostgreSQL adapter {} requires a non-empty lowercase `hephaestus.database_context`",
                package.name
            ),
        ));
    }
    valid_context
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
    package
        .metadata
        .get("hephaestus")
        .and_then(|metadata| metadata.get("postgres_adapter"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn visit_sources(
    root: &Path,
    directory: &Path,
    active: &BTreeSet<&str>,
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
            visit_sources(root, &path, active, diagnostics);
        } else if path.extension() == Some(OsStr::new("rs")) {
            validate_rust_source(root, relative, &path, active, diagnostics);
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
        || relative.starts_with("crates/hephaestus-dev/tests/fixtures")
}

fn validate_rust_source(
    root: &Path,
    relative: &Path,
    path: &Path,
    active: &BTreeSet<&str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Ok(source) = fs::read_to_string(path) else {
        return;
    };
    let Ok(file) = syn::parse_file(&source) else {
        return;
    };
    let imports = SqlImports::collect(&file);
    let mut visitor = SqlVisitor {
        repository_root: root,
        package_root: path
            .ancestors()
            .find(|ancestor| ancestor.join("Cargo.toml").is_file())
            .unwrap_or(path),
        source_path: path,
        path: relative,
        active,
        diagnostics,
        imports,
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

struct SqlVisitor<'a> {
    repository_root: &'a Path,
    package_root: &'a Path,
    source_path: &'a Path,
    path: &'a Path,
    active: &'a BTreeSet<&'a str>,
    diagnostics: &'a mut Vec<Diagnostic>,
    imports: SqlImports,
}

impl SqlVisitor<'_> {
    fn validate_argument(&mut self, argument: Option<&Expr>, kind: QueryKind) {
        let Some(argument) = argument else {
            return;
        };
        match argument {
            Expr::Lit(literal) => {
                if let Lit::Str(sql) = &literal.lit {
                    match kind {
                        QueryKind::Inline | QueryKind::Builder => {
                            self.validate_schema_sql(&sql.value(), None);
                        }
                        QueryKind::File => {
                            self.validate_sql_file(&sql.value(), self.package_root);
                        }
                    }
                }
            }
            Expr::Macro(expression) if expression.mac.path.is_ident("include_str") => {
                if let Ok(path) = syn::parse2::<syn::LitStr>(expression.mac.tokens.clone()) {
                    let source_root = self.source_path.parent().unwrap_or(self.source_path);
                    self.validate_sql_file(&path.value(), source_root);
                }
            }
            _ if self.active.contains(STATIC_RULE) => self.diagnostics.push(Diagnostic::new(
                STATIC_RULE,
                format!(
                    "SQLx query in {} must receive a static string literal or include_str! source",
                    self.path.display()
                ),
            )),
            _ => {}
        }
    }

    fn validate_sql_file(&mut self, path: &str, base: &Path) {
        let target = base.join(path);
        let Ok(sql) = fs::read_to_string(&target) else {
            return;
        };
        self.validate_schema_sql(&sql, Some(&target));
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
    use super::{RULES, audit, contains_schema_sql};
    use crate::checks::architecture::{CargoDependency, CargoMetadata, CargoPackage};
    use serde_json::json;
    use std::path::{Path, PathBuf};

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
        assert!(audit(&root, &metadata).is_empty());
    }

    #[test]
    fn valid_adapter_is_a_capability_firewall_and_owns_only_static_queries() {
        let root = fixture("valid");
        let adapter = package(
            &root,
            "adapter",
            json!({"hephaestus": {"postgres_adapter": true, "database_context": "fixture"}}),
            vec![sqlx_dependency()],
        );
        let application = package(
            &root,
            "application",
            serde_json::Value::Null,
            vec![path_dependency("adapter", root.join("adapter"))],
        );
        let metadata = CargoMetadata {
            workspace_members: vec![adapter.id.clone(), application.id.clone()],
            packages: vec![adapter, application],
            workspace_root: root.clone(),
        };
        assert!(audit(&root, &metadata).is_empty());
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
            json!({"hephaestus": {"postgres_adapter": true}}),
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
        let counts = audit(&root, &metadata);
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
}

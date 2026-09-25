//! Static application-role transaction-context checks.

use super::{CargoMetadata, Diagnostic};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::{
    Expr, ExprCall, ExprLit, ExprMethodCall, ExprPath, File, ItemFn, ItemImpl, Lit, Pat,
    spanned::Spanned, visit::Visit,
};

pub(super) const RULE: &str = "DB-RLS-CONTEXT-REQUIRED";

const CANONICAL_HELPERS: [&str; 2] = [
    "begin_actor_transaction",
    "begin_repeatable_read_actor_transaction",
];
#[derive(Clone, Copy)]
struct ResolverSpec {
    name: &'static str,
    migration: &'static str,
}

const RESOLVER_SPECS: [ResolverSpec; 6] = [
    ResolverSpec {
        name: "authenticate_human_browser_session",
        migration: "migrations/0085_human_browser_sessions.sql",
    },
    ResolverSpec {
        name: "authenticate_ui_browser_session",
        migration: "migrations/0090_ui_browser_session_authentication.sql",
    },
    ResolverSpec {
        name: "resolve_active_ui_generation_host",
        migration: "migrations/0092_ui_browser_host_and_resource_reads.sql",
    },
    ResolverSpec {
        name: "resolve_ui_browser_resource",
        migration: "migrations/0092_ui_browser_host_and_resource_reads.sql",
    },
    ResolverSpec {
        name: "resolve_ui_browser_repository_target_context",
        migration: "migrations/0098_ui_browser_target_context.sql",
    },
    ResolverSpec {
        name: "resolve_ui_browser_repository_git_context",
        migration: "migrations/0097_ui_repository_git_audit_context.sql",
    },
];

#[derive(Clone, Copy)]
struct PoolBinding {
    source: &'static str,
    owner: &'static str,
    field: &'static str,
}

#[derive(Clone, Copy)]
struct PoolField {
    source: &'static str,
    owner: &'static str,
    field: &'static str,
    application_role: bool,
}

// These are the exact application-role fields established by the app
// composition. Generic `pool` fields are listed explicitly rather than
// treating every PostgreSQL pool as an application-role pool.
const PRODUCTION_BINDINGS: [PoolBinding; 6] = [
    PoolBinding {
        source: "crates/heph-core/identity/postgres/src/session.rs",
        owner: "PostgresBrowserSessionStore",
        field: "application_pool",
    },
    PoolBinding {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser.rs",
        owner: "PgUiBrowserSessionStore",
        field: "app_pool",
    },
    PoolBinding {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
        owner: "PgUiGenerationHostResolver",
        field: "app_pool",
    },
    PoolBinding {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
        owner: "PgUiBrowserServingStore",
        field: "app_pool",
    },
    PoolBinding {
        source: "crates/heph-core/forge/release/postgres/src/ui_installation_navigation.rs",
        owner: "PgUiInstallationNavigator",
        field: "pool",
    },
    PoolBinding {
        source: "crates/heph-core/gateway/postgres/src/service_log_reader.rs",
        owner: "PostgresGatewayServiceLogReader",
        field: "pool",
    },
];

const PRODUCTION_POOL_FIELDS: [PoolField; 8] = [
    PoolField {
        source: "crates/heph-core/identity/postgres/src/session.rs",
        owner: "PostgresBrowserSessionStore",
        field: "worker_pool",
        application_role: false,
    },
    PoolField {
        source: "crates/heph-core/identity/postgres/src/session.rs",
        owner: "PostgresBrowserSessionStore",
        field: "application_pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser.rs",
        owner: "PgUiBrowserSessionStore",
        field: "worker_pool",
        application_role: false,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser.rs",
        owner: "PgUiBrowserSessionStore",
        field: "app_pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
        owner: "PgUiGenerationHostResolver",
        field: "app_pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
        owner: "PgUiBrowserServingStore",
        field: "app_pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_installation_navigation.rs",
        owner: "PgUiInstallationNavigator",
        field: "pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/gateway/postgres/src/service_log_reader.rs",
        owner: "PostgresGatewayServiceLogReader",
        field: "pool",
        application_role: true,
    },
];

const PRODUCTION_SOURCES: [&str; 5] = [
    "crates/heph-core/identity/postgres/src/session.rs",
    "crates/heph-core/forge/release/postgres/src/ui_browser.rs",
    "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
    "crates/heph-core/forge/release/postgres/src/ui_installation_navigation.rs",
    "crates/heph-core/gateway/postgres/src/service_log_reader.rs",
];

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

fn validate_file(
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

fn validate_pool_inventory(
    file: &File,
    relative: &Path,
    inventory: &[PoolField],
    bindings: &[PoolBinding],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for binding in inventory
        .iter()
        .filter(|entry| entry.application_role && entry.source == relative.to_string_lossy())
    {
        if !bindings
            .iter()
            .any(|entry| entry.owner == binding.owner && entry.field == binding.field)
        {
            diagnostics.push(Diagnostic::new(
                RULE,
                format!(
                    "{}::{}::{} is missing from the application-role pool inventory",
                    binding.source, binding.owner, binding.field
                ),
            ));
        }
    }
    for item in &file.items {
        let syn::Item::Struct(item) = item else {
            continue;
        };
        let owner = item.ident.to_string();
        let syn::Fields::Named(fields) = &item.fields else {
            continue;
        };
        for field in fields
            .named
            .iter()
            .filter(|field| is_pg_pool_type(&field.ty))
        {
            let Some(name) = field.ident.as_ref().map(ToString::to_string) else {
                continue;
            };
            if !inventory.iter().any(|entry| {
                entry.source == relative.to_string_lossy()
                    && entry.owner == owner
                    && entry.field == name
            }) {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "{}::{owner}::{name} is an unclassified PostgreSQL pool field; declare its role explicitly",
                        relative.display()
                    ),
                ));
            }
        }
    }
}

fn is_pg_pool_type(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "PgPool")
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

struct FunctionScanner<'a> {
    relative: &'a Path,
    source_path: &'a Path,
    bindings: &'a [PoolBinding],
    repository_root: &'a Path,
    owner: Option<&'a str>,
    item: &'a str,
    diagnostics: &'a mut Vec<Diagnostic>,
    canonical_transactions: BTreeSet<String>,
    application_transactions: BTreeSet<String>,
    resolver_transactions: BTreeSet<String>,
    verified_transactions: BTreeSet<String>,
    saw_application_begin: bool,
    has_complete_verified_helper: bool,
}

impl<'a> FunctionScanner<'a> {
    const fn new(
        relative: &'a Path,
        source_path: &'a Path,
        bindings: &'a [PoolBinding],
        repository_root: &'a Path,
        owner: Option<&'a str>,
        item: &'a str,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        Self {
            relative,
            source_path,
            bindings,
            repository_root,
            owner,
            item,
            diagnostics,
            canonical_transactions: BTreeSet::new(),
            application_transactions: BTreeSet::new(),
            resolver_transactions: BTreeSet::new(),
            verified_transactions: BTreeSet::new(),
            saw_application_begin: false,
            has_complete_verified_helper: false,
        }
    }

    fn report(&mut self, message: impl Into<String>, span: proc_macro2::Span) {
        self.diagnostics.push(Diagnostic::new(
            RULE,
            format!(
                "{}#{}: {}",
                self.relative.display(),
                self.item_path(),
                message.into()
            ),
        ));
        let _ = span;
    }

    fn item_path(&self) -> String {
        self.owner.map_or_else(
            || self.item.to_owned(),
            |owner| format!("{owner}::{}", self.item),
        )
    }

    fn is_application_field(&self, expression: &Expr) -> bool {
        let Some(owner) = self.owner else {
            return false;
        };
        self.bindings.iter().any(|binding| {
            binding.owner == owner && field_expression_matches(expression, binding.field)
        })
    }

    fn transaction_name(expression: &Expr) -> Option<String> {
        match expression {
            Expr::Reference(reference) => Self::transaction_name(&reference.expr),
            Expr::Unary(unary) => Self::transaction_name(&unary.expr),
            Expr::Path(path) => path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string()),
            _ => None,
        }
    }

    fn query_sql(&self, expression: &Expr) -> Option<String> {
        let argument = query_argument(expression)?;
        match argument {
            Expr::Lit(ExprLit {
                lit: Lit::Str(value),
                ..
            }) => Some(value.value()),
            Expr::Macro(mac) if mac.mac.path.is_ident("include_str") => {
                let literal = syn::parse2::<syn::LitStr>(mac.mac.tokens.clone()).ok()?;
                let path = self.source_path.parent()?.join(literal.value());
                fs::read_to_string(path).ok()
            }
            _ => None,
        }
    }

    fn is_context_query(&self, sql: Option<&str>) -> bool {
        self.item == "set_verified_actor_context"
            && sql.is_some_and(|sql| {
                sql.contains("hephaestus.actor_id")
                    && sql.contains("hephaestus.subject_type")
                    && sql.contains("hephaestus.request_id")
                    && sql.contains("hephaestus.occurrence_id")
            })
    }

    fn check_query(&mut self, call: &ExprMethodCall) {
        let Some(executor) = call.args.last() else {
            return;
        };
        let sql = self.query_sql(&call.receiver);
        let method = call.method.to_string();
        if !is_executor_method(&method) {
            return;
        }
        if self.is_application_field(executor) {
            if sql
                .as_deref()
                .is_some_and(|sql| is_allowlisted_resolver(sql, self.repository_root))
            {
                return;
            }
            self.report(
                "direct application-role pool query requires a canonical actor transaction or an exact security-definer resolver",
                call.span(),
            );
            return;
        }
        let Some(transaction) = Self::transaction_name(executor) else {
            return;
        };
        if self.is_context_query(sql.as_deref()) {
            self.has_complete_verified_helper = true;
            self.verified_transactions.insert(transaction);
            return;
        }
        if sql
            .as_deref()
            .is_some_and(|sql| is_allowlisted_resolver(sql, self.repository_root))
        {
            if self.application_transactions.contains(&transaction) {
                self.resolver_transactions.insert(transaction);
            }
            return;
        }
        if self.application_transactions.contains(&transaction)
            && !self.canonical_transactions.contains(&transaction)
            && !self.verified_transactions.contains(&transaction)
        {
            self.report(
                "application-role query runs before verified actor context",
                call.span(),
            );
        }
    }

    fn finish(&mut self) {
        if self.saw_application_begin {
            for transaction in self.application_transactions.clone() {
                if !self.canonical_transactions.contains(&transaction)
                    && !self.resolver_transactions.contains(&transaction)
                {
                    self.report(
                        "application-role transaction must use begin_actor_transaction or an exact bootstrap resolver",
                        proc_macro2::Span::call_site(),
                    );
                }
            }
        }
        if self.item == "set_verified_actor_context" && !self.has_complete_verified_helper {
            self.report(
                "verified actor context must set actor, subject, request, and occurrence settings",
                proc_macro2::Span::call_site(),
            );
        }
    }
}

impl<'ast> Visit<'ast> for FunctionScanner<'_> {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        let Some(Pat::Ident(pattern)) = Some(&local.pat) else {
            syn::visit::visit_local(self, local);
            return;
        };
        let name = pattern.ident.to_string();
        if let Some(initializer) = &local.init {
            if canonical_helper(initializer.expr.as_ref()) {
                self.canonical_transactions.insert(name.clone());
            }
            if application_begin(initializer.expr.as_ref(), self) {
                self.application_transactions.insert(name);
                self.saw_application_begin = true;
            }
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if path_last_ident(&call.func).is_some_and(|name| name == "set_verified_actor_context") {
            if let Some(transaction) = call.args.first().and_then(Self::transaction_name) {
                // The helper's own item is checked separately for all four
                // settings; this call marks the transaction for ordering.
                self.verified_transactions.insert(transaction);
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        self.check_query(call);
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        for statement in &block.stmts {
            self.visit_stmt(statement);
        }
        self.finish();
    }
}

fn field_expression_matches(expression: &Expr, field: &str) -> bool {
    matches!(expression, Expr::Reference(reference) if field_expression_matches(&reference.expr, field))
        || matches!(expression, Expr::Field(field_expression) if matches!(&field_expression.member, syn::Member::Named(name) if name == field)
            && matches!(&*field_expression.base, Expr::Path(path) if path.path.is_ident("self")))
}

fn application_begin(expression: &Expr, scanner: &FunctionScanner<'_>) -> bool {
    let expression = peel_expression(expression);
    let Expr::MethodCall(call) = expression else {
        return false;
    };
    call.method == "begin" && scanner.is_application_field(&call.receiver)
}

fn canonical_helper(expression: &Expr) -> bool {
    let expression = peel_expression(expression);
    let Expr::Call(call) = expression else {
        return false;
    };
    path_last_ident(&call.func).is_some_and(|name| CANONICAL_HELPERS.contains(&name.as_str()))
}

fn peel_expression(expression: &Expr) -> &Expr {
    match expression {
        Expr::Await(awaited) => peel_expression(&awaited.base),
        Expr::Try(tried) => peel_expression(&tried.expr),
        Expr::Paren(parenthesized) => peel_expression(&parenthesized.expr),
        _ => expression,
    }
}

fn query_argument(expression: &Expr) -> Option<&Expr> {
    match expression {
        Expr::Call(call) if is_query_constructor(&call.func) => call.args.first(),
        Expr::MethodCall(call) => query_argument(&call.receiver),
        _ => None,
    }
}

fn is_query_constructor(expression: &Expr) -> bool {
    path_last_ident(expression)
        .is_some_and(|name| matches!(name.as_str(), "query" | "query_as" | "query_scalar"))
}

fn is_executor_method(method: &str) -> bool {
    matches!(
        method,
        "execute" | "fetch_one" | "fetch_optional" | "fetch_all" | "fetch" | "fetch_many"
    )
}

fn is_allowlisted_resolver(sql: &str, repository_root: &Path) -> bool {
    let sql = strip_sql_comments(sql);
    let normalized = collapse_sql_whitespace(&sql).to_ascii_lowercase();
    if !normalized.starts_with("select ") || normalized.contains(';') {
        return false;
    }
    if [
        " with ",
        " join ",
        " where ",
        " union ",
        " group ",
        " order ",
        " limit ",
        " offset ",
        " having ",
        " into ",
        " returning ",
        " insert ",
        " update ",
        " delete ",
    ]
    .iter()
    .any(|keyword| normalized.contains(keyword))
    {
        return false;
    }
    let Some(from) = token_position(&normalized, "from") else {
        return false;
    };
    let prefix = normalized[..from].trim_end();
    if !prefix.starts_with("select ") || prefix[7..].contains("select") {
        return false;
    }
    let suffix = normalized[from + 4..].trim_start();
    let Some(spec) = RESOLVER_SPECS.iter().find(|spec| {
        let bare = format!("{}(", spec.name);
        let qualified = format!("public.{}(", spec.name);
        suffix.starts_with(&bare) || suffix.starts_with(&qualified)
    }) else {
        return false;
    };
    let function_start = if suffix.starts_with("public.") {
        spec.name.len() + 7
    } else {
        spec.name.len()
    };
    let Some(open) = suffix[function_start..].find('(') else {
        return false;
    };
    let open = function_start + open;
    let Some(close) = suffix.rfind(')') else {
        return false;
    };
    if close < open || !suffix[close + 1..].trim().is_empty() {
        return false;
    }
    if suffix[open + 1..close].contains("select") {
        return false;
    }
    migration_is_security_definer(repository_root, *spec)
}

fn token_position(sql: &str, token: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(index) = sql[offset..].find(token) {
        let start = offset + index;
        let end = start + token.len();
        let before_ok = start == 0 || !sql.as_bytes()[start - 1].is_ascii_alphanumeric();
        let after_ok = end == sql.len() || !sql.as_bytes()[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return Some(start);
        }
        offset = end;
    }
    None
}

fn collapse_sql_whitespace(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_sql_comments(sql: &str) -> String {
    let mut output = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"--") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            output.push(' ');
        } else if bytes[index..].starts_with(b"/*") {
            index += 2;
            while index + 1 < bytes.len() && !bytes[index..].starts_with(b"*/") {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            output.push(' ');
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
}

fn migration_is_security_definer(root: &Path, resolver: ResolverSpec) -> bool {
    let Ok(source) = fs::read_to_string(root.join(resolver.migration)) else {
        return false;
    };
    let source = source.to_ascii_lowercase();
    let Some(start) = source.find(&format!("create function {}", resolver.name)) else {
        return false;
    };
    let end = source[start + 1..]
        .find("create function ")
        .map_or(source.len(), |offset| start + 1 + offset);
    source[start..end].contains("security definer")
}

fn path_last_ident(expression: &Expr) -> Option<String> {
    let Expr::Path(ExprPath { path, .. }) = expression else {
        return None;
    };
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
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

#[cfg(test)]
mod tests {
    use super::{PoolBinding, PoolField, RULE, validate_file, validate_pool_inventory};
    use std::{fs, path::Path};

    fn scan(name: &str, binding: PoolBinding) -> Vec<super::super::Diagnostic> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/db-rls");
        let relative = Path::new(name).join("src/lib.rs");
        let path = root.join(&relative);
        let source = fs::read_to_string(&path).expect("fixture source");
        let file = syn::parse_file(&source).expect("fixture Rust");
        let mut diagnostics = Vec::new();
        validate_file(&file, &relative, &path, &[binding], &root, &mut diagnostics);
        diagnostics
    }

    fn binding() -> PoolBinding {
        PoolBinding {
            source: "fixture",
            owner: "AppStore",
            field: "app_pool",
        }
    }

    #[test]
    fn an_unlisted_pool_field_is_rejected_by_the_structural_inventory() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/db-rls");
        let relative = Path::new("invalid/new_pool_field/src/lib.rs");
        let path = root.join(relative);
        let source = fs::read_to_string(&path).expect("fixture source");
        let file = syn::parse_file(&source).expect("fixture Rust");
        let mut diagnostics = Vec::new();
        let inventory = [PoolField {
            source: "invalid/new_pool_field/src/lib.rs",
            owner: "AppStore",
            field: "app_pool",
            application_role: true,
        }];
        let bindings = [PoolBinding {
            source: "invalid/new_pool_field/src/lib.rs",
            owner: "AppStore",
            field: "app_pool",
        }];
        validate_pool_inventory(&file, relative, &inventory, &bindings, &mut diagnostics);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == RULE)
        );
    }

    #[test]
    fn valid_context_fixtures_are_clean() {
        for fixture in [
            "valid/canonical",
            "valid/repeatable",
            "valid/bootstrap",
            "valid/worker",
        ] {
            assert!(
                scan(fixture, binding())
                    .iter()
                    .all(|diagnostic| diagnostic.rule_id != RULE),
                "{fixture}"
            );
        }
    }

    #[test]
    fn invalid_context_fixtures_report_the_rule() {
        for fixture in [
            "invalid/direct",
            "invalid/unscoped",
            "invalid/misleading",
            "invalid/pre_context",
            "invalid/incomplete_helper",
            "invalid/comment_bypass",
            "invalid/unbacked_resolver",
        ] {
            assert!(
                scan(fixture, binding())
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == RULE),
                "{fixture}"
            );
        }
    }
}

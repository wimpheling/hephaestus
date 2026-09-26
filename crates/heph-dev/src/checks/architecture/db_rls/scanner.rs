//! Function-level transaction and query context analysis.

use super::{Diagnostic, RULE, inventory::PoolBinding, resolver::is_allowlisted_resolver};
use std::{collections::BTreeSet, fs, path::Path};
use syn::{Expr, ExprCall, ExprLit, ExprMethodCall, Lit, Pat, spanned::Spanned, visit::Visit};

const CANONICAL_HELPERS: [&str; 2] = [
    "begin_actor_transaction",
    "begin_repeatable_read_actor_transaction",
];

pub(super) struct FunctionScanner<'a> {
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
    pub(super) const fn new(
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

fn path_last_ident(expression: &Expr) -> Option<String> {
    let Expr::Path(path) = expression else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

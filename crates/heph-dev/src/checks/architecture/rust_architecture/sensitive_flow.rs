//! Syntax-level sensitive value flow detection.

use super::{
    Diagnostic,
    rules::{
        call_sink_rule, expression_path_names, is_opaque_conversion, is_sensitive_output_type,
        is_sensitive_request_field, is_supported_sensitive_macro, macro_sink_rule,
        method_sink_rule, report, token_contains_name,
    },
};
use std::{collections::BTreeMap, path::Path};
use syn::{
    Expr, ExprCall, ExprField, ExprMacro, ExprMethodCall, ExprStruct, Ident, spanned::Spanned,
    visit::Visit,
};

#[derive(Clone, Copy)]
enum SensitiveValue {
    Plain {
        origin: &'static str,
        source_line: usize,
    },
    RequestRoot,
    Opaque,
}

const fn local_binding_name(pattern: &syn::Pat) -> Option<&Ident> {
    match pattern {
        syn::Pat::Ident(pattern) => Some(&pattern.ident),
        _ => None,
    }
}

const fn is_plain_sensitive(value: SensitiveValue) -> bool {
    matches!(value, SensitiveValue::Plain { .. })
}

const fn plain_source_line(value: SensitiveValue) -> Option<usize> {
    match value {
        SensitiveValue::Plain { source_line, .. } => Some(source_line),
        SensitiveValue::RequestRoot | SensitiveValue::Opaque => None,
    }
}

fn span_line<T: Spanned>(node: &T) -> usize {
    node.span().start().line
}

pub(super) fn detect_sensitive_flows(
    path: &Path,
    source: &str,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Ok(file) = syn::parse_file(source) else {
        return;
    };
    let mut visitor = SensitiveFlowVisitor {
        path,
        active,
        diagnostics,
        bindings: BTreeMap::new(),
        item_name: None,
    };
    visitor.visit_file(&file);
}

struct SensitiveFlowVisitor<'a> {
    path: &'a Path,
    active: &'a [&'a str],
    diagnostics: &'a mut Vec<Diagnostic>,
    bindings: BTreeMap<String, SensitiveValue>,
    item_name: Option<String>,
}

impl<'ast> Visit<'ast> for SensitiveFlowVisitor<'_> {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let bindings = std::mem::take(&mut self.bindings);
        let item_name = self.item_name.take();
        self.item_name = Some(item.sig.ident.to_string());
        syn::visit::visit_item_fn(self, item);
        self.bindings = bindings;
        self.item_name = item_name;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let bindings = std::mem::take(&mut self.bindings);
        let item_name = self.item_name.take();
        self.item_name = Some(item.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, item);
        self.bindings = bindings;
        self.item_name = item_name;
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Some(name) = local_binding_name(&local.pat)
            && let Some(initializer) = &local.init
            && let Some(value) = self.sensitive_expr(&initializer.expr)
        {
            self.bindings.insert(name.to_string(), value);
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_expr_assign(&mut self, assignment: &'ast syn::ExprAssign) {
        if let Expr::Path(path) = assignment.left.as_ref()
            && let Some(name) = path.path.get_ident()
        {
            if let Some(value) = self.sensitive_expr(&assignment.right) {
                self.bindings.insert(name.to_string(), value);
            } else {
                self.bindings.remove(&name.to_string());
            }
        }
        syn::visit::visit_expr_assign(self, assignment);
    }

    fn visit_expr_macro(&mut self, expression: &'ast ExprMacro) {
        self.visit_macro(&expression.mac);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let macro_name = mac
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        if is_supported_sensitive_macro(&mac.path, &macro_name)
            && self.macro_contains_sensitive(&mac.tokens.to_string())
        {
            let sink_line = span_line(mac);
            self.report_sink(
                &macro_name,
                macro_sink_rule(&mac.path, &macro_name),
                self.macro_source_line(&mac.tokens.to_string())
                    .unwrap_or(sink_line),
                sink_line,
            );
        }
    }

    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if let Some(rule) = call_sink_rule(&expression.func)
            && expression.args.iter().any(|argument| {
                self.sensitive_expr(argument)
                    .is_some_and(is_plain_sensitive)
            })
        {
            let sink = expression_path_names(&expression.func);
            let sink_line = span_line(expression);
            self.report_sink(
                if sink.is_empty() { "call" } else { &sink },
                rule,
                expression
                    .args
                    .iter()
                    .find_map(|argument| self.sensitive_expr(argument).and_then(plain_source_line))
                    .unwrap_or(sink_line),
                sink_line,
            );
        }
        syn::visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast ExprMethodCall) {
        if let Some(rule) = method_sink_rule(&expression.method, &expression.receiver)
            && (self
                .sensitive_expr(&expression.receiver)
                .is_some_and(is_plain_sensitive)
                || expression.args.iter().any(|argument| {
                    self.sensitive_expr(argument)
                        .is_some_and(is_plain_sensitive)
                }))
        {
            let sink_line = span_line(expression);
            self.report_sink(
                &expression.method.to_string(),
                rule,
                self.sensitive_expr(&expression.receiver)
                    .and_then(plain_source_line)
                    .or_else(|| {
                        expression.args.iter().find_map(|argument| {
                            self.sensitive_expr(argument).and_then(plain_source_line)
                        })
                    })
                    .unwrap_or(sink_line),
                sink_line,
            );
        }
        syn::visit::visit_expr_method_call(self, expression);
    }

    fn visit_expr_struct(&mut self, expression: &'ast ExprStruct) {
        let is_sink = expression
            .path
            .segments
            .last()
            .is_some_and(|segment| is_sensitive_output_type(&segment.ident));
        if is_sink
            && expression.fields.iter().any(|field| {
                self.sensitive_expr(&field.expr)
                    .is_some_and(is_plain_sensitive)
            })
        {
            let sink_line = span_line(expression);
            self.report_sink(
                "struct field",
                "SEC-NO-SENSITIVE-LOG-ARGUMENTS",
                expression
                    .fields
                    .iter()
                    .find_map(|field| self.sensitive_expr(&field.expr).and_then(plain_source_line))
                    .unwrap_or(sink_line),
                sink_line,
            );
        }
        syn::visit::visit_expr_struct(self, expression);
    }
}

impl SensitiveFlowVisitor<'_> {
    fn sensitive_expr(&self, expression: &Expr) -> Option<SensitiveValue> {
        match expression {
            Expr::Field(field) => self.sensitive_field(field),
            Expr::Path(path) => path.path.get_ident().and_then(|name| {
                if name == "request" {
                    Some(SensitiveValue::RequestRoot)
                } else {
                    self.bindings.get(&name.to_string()).copied()
                }
            }),
            Expr::Reference(reference) => self.sensitive_expr(&reference.expr),
            Expr::Paren(parenthesized) => self.sensitive_expr(&parenthesized.expr),
            Expr::Group(group) => self.sensitive_expr(&group.expr),
            Expr::Cast(cast) => self.sensitive_expr(&cast.expr),
            Expr::Unary(unary) => self.sensitive_expr(&unary.expr),
            Expr::Call(call) => {
                let argument_values = call
                    .args
                    .iter()
                    .filter_map(|argument| self.sensitive_expr(argument))
                    .collect::<Vec<_>>();
                if argument_values
                    .iter()
                    .any(|value| is_plain_sensitive(*value))
                {
                    if is_opaque_conversion(&call.func) {
                        Some(SensitiveValue::Opaque)
                    } else {
                        Some(SensitiveValue::Plain {
                            origin: "sensitive request field",
                            source_line: argument_values
                                .iter()
                                .find_map(|value| plain_source_line(*value))
                                .unwrap_or_else(|| span_line(call)),
                        })
                    }
                } else if !argument_values.is_empty() {
                    Some(SensitiveValue::Opaque)
                } else {
                    None
                }
            }
            Expr::MethodCall(method) => self.sensitive_expr(&method.receiver),
            _ => None,
        }
    }

    fn sensitive_field(&self, field: &ExprField) -> Option<SensitiveValue> {
        let field_name = match &field.member {
            syn::Member::Named(name) => name.to_string(),
            syn::Member::Unnamed(_) => return self.sensitive_expr(&field.base),
        };
        if is_sensitive_request_field(&field_name) && self.is_request_rooted(&field.base) {
            return Some(SensitiveValue::Plain {
                origin: "sensitive request field",
                source_line: span_line(field),
            });
        }
        if self.is_request_rooted(&field.base) {
            return Some(SensitiveValue::RequestRoot);
        }
        self.sensitive_expr(&field.base)
    }

    fn is_request_rooted(&self, expression: &Expr) -> bool {
        match expression {
            Expr::Path(path) => path.path.get_ident().is_some_and(|name| {
                name == "request"
                    || self
                        .bindings
                        .get(&name.to_string())
                        .is_some_and(|value| matches!(value, SensitiveValue::RequestRoot))
            }),
            Expr::Field(field) => self.is_request_rooted(&field.base),
            Expr::Reference(reference) => self.is_request_rooted(&reference.expr),
            Expr::Paren(parenthesized) => self.is_request_rooted(&parenthesized.expr),
            Expr::Group(group) => self.is_request_rooted(&group.expr),
            _ => false,
        }
    }

    fn macro_contains_sensitive(&self, tokens: &str) -> bool {
        self.bindings
            .keys()
            .filter_map(|name| self.bindings.get(name).map(|value| (name, value)))
            .any(|(name, value)| {
                matches!(value, SensitiveValue::Plain { .. }) && token_contains_name(tokens, name)
            })
    }

    fn macro_source_line(&self, tokens: &str) -> Option<usize> {
        self.bindings
            .iter()
            .find(|(name, value)| {
                matches!(value, SensitiveValue::Plain { .. }) && token_contains_name(tokens, name)
            })
            .and_then(|(_, value)| plain_source_line(*value))
    }

    fn report_sink(
        &mut self,
        sink: &str,
        rule: &'static str,
        source_line: usize,
        sink_line: usize,
    ) {
        if !self.active.contains(&rule) {
            return;
        }
        let origin = self
            .bindings
            .values()
            .find_map(|value| match value {
                SensitiveValue::Plain { origin, .. } => Some(*origin),
                SensitiveValue::RequestRoot | SensitiveValue::Opaque => None,
            })
            .unwrap_or("sensitive request field");
        report(
            self.diagnostics,
            rule,
            self.path,
            &format!(
                "{origin} source at line {source_line}; flows into {sink} sink in {}; line {sink_line}; redact or omit the value",
                self.item_name.as_deref().unwrap_or("module scope"),
            ),
        );
    }
}

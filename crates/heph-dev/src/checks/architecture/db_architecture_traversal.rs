use super::{Expr, File, QueryKind, SqlImports, SqlVisitor};
use std::collections::BTreeSet;
use syn::{ExprCall, ExprMacro, punctuated::Punctuated, token::Comma, visit::Visit};

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

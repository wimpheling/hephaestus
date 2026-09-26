//! Explicit request-budget checks for production RPC handlers.

use super::module_graph;
use crate::checks::architecture::Diagnostic;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use syn::{Expr, ExprCall, ExprPath, File, ImplItem, Item, ItemFn, ItemImpl, spanned::Spanned};

#[path = "deadline_ast.rs"]
mod ast;
#[path = "deadline_stream.rs"]
mod stream;

pub(super) const RULE: &str = "RPC-DEADLINE-CANCELLATION-PROPAGATION";

#[derive(Clone)]
struct SourceFile {
    path: PathBuf,
    source: String,
    syntax: File,
}

struct Handler {
    path: PathBuf,
    source: String,
    line: usize,
}

struct ServiceMethod {
    name: String,
    source: String,
    line: usize,
    stream: bool,
    delegation: Option<(String, String)>,
}

pub(super) fn validate(root: &Path, active: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if !active.contains(&RULE) {
        return;
    }
    let files = source_files(root);
    let handlers = handler_definitions(&files);
    for file in &files {
        for method in service_methods(file) {
            let Some((module, function)) = method.delegation else {
                check_handler(&file.path, &method, false, diagnostics);
                continue;
            };
            let key = format!("{module}::{function}");
            let Some(handler) = handlers.get(&key) else {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "{}:{} `{}` delegates to unresolved `{key}`; expose a concrete handler with RequestBudget propagation",
                        file.path.display(), method.line, method.name
                    ),
                ));
                continue;
            };
            let test_only = module_graph::is_test_only_source(root, &root.join(&handler.path));
            check_handler(
                &handler.path,
                &ServiceMethod {
                    name: format!("{}::{}", key, method.name),
                    source: handler.source.clone(),
                    line: handler.line,
                    stream: method.stream,
                    delegation: None,
                },
                test_only,
                diagnostics,
            );
        }
    }
    stream::validate(root, diagnostics);
}

fn source_files(root: &Path) -> Vec<SourceFile> {
    let mut files = Vec::new();
    collect_sources(root, &root.join("crates/heph-app/src/rpc"), &mut files);
    files
}

fn collect_sources(root: &Path, directory: &Path, files: &mut Vec<SourceFile>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            collect_sources(root, &path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(syntax) = syn::parse_file(&source) else {
                continue;
            };
            files.push(SourceFile {
                path: path.strip_prefix(root).unwrap_or(&path).to_owned(),
                source,
                syntax,
            });
        }
    }
}

fn handler_definitions(files: &[SourceFile]) -> BTreeMap<String, Handler> {
    let mut handlers = BTreeMap::new();
    for file in files {
        let module = file
            .path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        for item in &file.syntax.items {
            let Item::Fn(function) = item else { continue };
            if !is_super_async(function) {
                continue;
            }
            handlers.insert(
                format!("{module}::{}", function.sig.ident),
                Handler {
                    path: file.path.clone(),
                    source: ast::span_text(&file.source, function.span()),
                    line: function.sig.ident.span().start().line,
                },
            );
        }
    }
    handlers
}

fn service_methods(file: &SourceFile) -> Vec<ServiceMethod> {
    let mut methods = Vec::new();
    for item in &file.syntax.items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        if !is_service_impl(item_impl) {
            continue;
        }
        for item in &item_impl.items {
            let ImplItem::Fn(function) = item else {
                continue;
            };
            if function.sig.asyncness.is_none() {
                continue;
            }
            let source = ast::span_text(&file.source, function.span());
            methods.push(ServiceMethod {
                name: function.sig.ident.to_string(),
                stream: source.contains("ServiceStream"),
                delegation: delegation(&function.block),
                line: function.sig.ident.span().start().line,
                source,
            });
        }
    }
    methods
}

fn is_service_impl(item: &ItemImpl) -> bool {
    item.trait_
        .as_ref()
        .and_then(|(_, path, _)| path.segments.last())
        .is_some_and(|segment| segment.ident.to_string().ends_with("Service"))
}

fn is_super_async(function: &ItemFn) -> bool {
    function.sig.asyncness.is_some()
        && matches!(function.vis, syn::Visibility::Restricted(ref visibility)
            if visibility.path.segments.last().is_some_and(|segment| segment.ident == "super"))
}

fn delegation(block: &syn::Block) -> Option<(String, String)> {
    struct Find(Option<(String, String)>);
    impl<'ast> syn::visit::Visit<'ast> for Find {
        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if self.0.is_none() {
                if let Expr::Path(ExprPath { path, .. }) = &*call.func {
                    if path.segments.len() == 2
                        && call.args.first().is_some_and(|argument| {
                            matches!(argument, Expr::Path(value) if value.path.is_ident("self"))
                        })
                    {
                        self.0 = Some((
                            path.segments[0].ident.to_string(),
                            path.segments[1].ident.to_string(),
                        ));
                    }
                }
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let mut visitor = Find(None);
    syn::visit::Visit::visit_block(&mut visitor, block);
    visitor.0
}

fn check_handler(
    path: &Path,
    method: &ServiceMethod,
    test_only: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if test_only {
        return;
    }
    let budget_count = method
        .source
        .matches("RequestBudget::from_transport")
        .count();
    if budget_count != 1 {
        diagnostics.push(Diagnostic::new(RULE, format!(
            "{}:{} `{}` must create exactly one RequestBudget::from_transport; found {budget_count}; create one transport budget and pass it to every downstream operation",
            path.display(), method.line, method.name
        )));
    }
    let allow_bare_budget_helper = path.ends_with("crates/heph-app/src/rpc/secret/mutations.rs");
    let bounded_call = ast::has_budgeted_call(&method.source, allow_bare_budget_helper);
    if !bounded_call {
        diagnostics.push(Diagnostic::new(RULE, format!(
            "{}:{} `{}` has no approved budgeted downstream operation; wrap application, adapter, and receipt futures with the transport budget",
            path.display(), method.line, method.name
        )));
    }
    for raw in ast::raw_downstream_awaits(&method.source, allow_bare_budget_helper) {
        diagnostics.push(Diagnostic::new(RULE, format!(
            "{}:{} `{}` awaits `{raw}` outside a budget helper; use the same transport budget for downstream and receipt work",
            path.display(), method.line, method.name
        )));
    }
    if method.source.contains("tokio::spawn") && !ast::spawn_is_bounded(&method.source) {
        diagnostics.push(Diagnostic::new(RULE, format!(
            "{}:{} `{}` starts detached work without an approved cancellation/deadline path; capture the request budget and use a bounded producer helper",
            path.display(), method.line, method.name
        )));
    }
    if method.stream
        && ![
            "start_with_budget",
            "start_filtered_with_budget",
            "run_with_stream_budget",
            "response_stream",
            "stream_artifact",
        ]
        .iter()
        .any(|marker| method.source.contains(marker))
    {
        diagnostics.push(Diagnostic::new(RULE, format!(
            "{}:{} `{}` is a stream without an approved budgeted producer/receiver path; bind the stream to the transport budget",
            path.display(), method.line, method.name
        )));
    }
}

#[cfg(test)]
#[path = "deadline_tests.rs"]
mod tests;

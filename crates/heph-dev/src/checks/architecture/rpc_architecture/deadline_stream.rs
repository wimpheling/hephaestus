use super::ast;
use crate::checks::architecture::Diagnostic;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::spanned::Spanned;

const RULE: &str = "RPC-DEADLINE-CANCELLATION-PROPAGATION";

pub(super) fn validate(root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    let mut files = Vec::new();
    collect_sources(&root.join("crates/heph-app/src/rpc"), &mut files);
    for path in files {
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let production = source.split("#[cfg(test)]").next().unwrap_or(&source);
        let Ok(syntax) = syn::parse_file(production) else {
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let mut visitor = SpawnVisitor {
            source: production,
            path: relative,
            diagnostics,
        };
        syn::visit::Visit::visit_file(&mut visitor, &syntax);
    }
}

struct SpawnVisitor<'a> {
    source: &'a str,
    path: String,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl<'ast> syn::visit::Visit<'ast> for SpawnVisitor<'_> {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        let source = ast::span_text(self.source, function.span());
        if source.contains("tokio::spawn") && !ast::spawn_is_bounded(&source) {
            self.diagnostics.push(Diagnostic::new(
                RULE,
                format!(
                    "{}:{} detached stream work lacks an explicit budget/cancellation path or contains an unbounded send; pass RequestBudget or its cancellation token",
                    self.path,
                    function.sig.ident.span().start().line
                ),
            ));
        }
        syn::visit::visit_item_fn(self, function);
    }
}

fn collect_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            collect_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

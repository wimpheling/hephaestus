use proc_macro2::Span;
use syn::{Expr, ExprCall, ExprPath, ExprReference, ItemFn, spanned::Spanned};

const BUDGET_HELPERS: &[&str] = &[
    "run_with_budget",
    "run_with_stream_budget",
    "run_creation",
    "execute",
    "receipt",
];

pub(super) fn has_budgeted_call(source: &str, allow_bare_budget_helper: bool) -> bool {
    struct Find {
        found: bool,
        allow_bare: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for Find {
        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            self.found |= is_budgeted_call(call, self.allow_bare);
            syn::visit::visit_expr_call(self, call);
        }
    }
    let Ok(item) = syn::parse_str::<ItemFn>(source) else {
        return false;
    };
    let mut visitor = Find {
        found: false,
        allow_bare: allow_bare_budget_helper,
    };
    syn::visit::Visit::visit_item_fn(&mut visitor, &item);
    visitor.found
}

pub(super) fn is_budgeted_call(call: &ExprCall, allow_bare_budget_helper: bool) -> bool {
    let Some(path) = call_path(&call.func) else {
        return false;
    };
    let Some(name) = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
    else {
        return false;
    };
    if BUDGET_HELPERS.contains(&name.as_str()) {
        if matches!(name.as_str(), "execute" | "receipt") {
            let qualified = path
                .segments
                .iter()
                .any(|segment| segment.ident == "budget");
            let bare = allow_bare_budget_helper && path.segments.len() == 1;
            if !qualified && !bare {
                return false;
            }
        }
        return call.args.first().is_some_and(is_budget_arg);
    }
    match name.as_str() {
        "start_with_budget" | "start_filtered_with_budget" => {
            call.args.last().is_some_and(is_budget_arg)
        }
        "response_stream" => call.args.get(1).is_some_and(is_budget_arg),
        _ => false,
    }
}

const fn call_path(expr: &Expr) -> Option<&syn::Path> {
    match expr {
        Expr::Path(ExprPath { path, .. }) => Some(path),
        _ => None,
    }
}

fn is_budget_arg(expr: &Expr) -> bool {
    match expr {
        Expr::Path(path) => path.path.is_ident("budget"),
        Expr::Reference(ExprReference { expr, .. }) => is_budget_arg(expr),
        _ => false,
    }
}

pub(super) fn raw_downstream_awaits(source: &str, allow_bare_budget_helper: bool) -> Vec<String> {
    struct Find<'a> {
        source: &'a str,
        allow_bare: bool,
        raw: Vec<String>,
    }
    impl<'ast> syn::visit::Visit<'ast> for Find<'_> {
        fn visit_expr_await(&mut self, await_expr: &'ast syn::ExprAwait) {
            let base = span_text(self.source, await_expr.base.span());
            let bounded = matches!(&*await_expr.base, Expr::Call(call) if is_budgeted_call(call, self.allow_bare));
            let downstream = [
                "application.",
                "event_application.",
                "forge.",
                "ui_installations.",
                "ui_navigator.",
                "service_logs.",
                "pool.",
                "receipts.",
                "service.execute(",
                "self.execute(",
                "mutation_receipt(",
                "identity_receipt(",
                "Repository::new(",
                ".operate(",
                ".allocate(",
            ]
            .iter()
            .any(|marker| base.contains(marker));
            if downstream && !bounded {
                self.raw
                    .push(base.lines().next().unwrap_or_default().to_owned());
            }
            syn::visit::visit_expr_await(self, await_expr);
        }
    }
    let Ok(item) = syn::parse_str::<ItemFn>(source) else {
        return Vec::new();
    };
    let mut visitor = Find {
        source,
        allow_bare: allow_bare_budget_helper,
        raw: Vec::new(),
    };
    syn::visit::Visit::visit_item_fn(&mut visitor, &item);
    visitor.raw
}

pub(super) fn spawn_is_bounded(source: &str) -> bool {
    enum SpawnState {
        None,
        Bounded,
        Unbounded,
    }
    struct Find<'a> {
        source: &'a str,
        in_spawn: bool,
        spawn_state: SpawnState,
        raw_stream_wait: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for Find<'_> {
        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            let is_spawn = call_path(&call.func)
                .and_then(|path| path.segments.last())
                .is_some_and(|segment| segment.ident == "spawn");
            if is_spawn {
                let text = span_text(self.source, call.span());
                let bounded = (text.contains("budget")
                    && (text.contains("run_with_stream_budget")
                        || text.contains("producer::run")
                        || text.contains("produce(")
                        || text.contains("cancellation")))
                    || (text.contains("cancellation") && text.contains("produce("));
                self.spawn_state = match (&self.spawn_state, bounded) {
                    (SpawnState::Unbounded, _) | (_, false) => SpawnState::Unbounded,
                    (SpawnState::None | SpawnState::Bounded, true) => SpawnState::Bounded,
                };
                let previous = self.in_spawn;
                self.in_spawn = true;
                syn::visit::visit_expr_call(self, call);
                self.in_spawn = previous;
                return;
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_await(&mut self, await_expr: &'ast syn::ExprAwait) {
            if self.in_spawn {
                let text = span_text(self.source, await_expr.base.span());
                let raw_wait = [
                    ".send(",
                    ".send::<",
                    ".recv(",
                    "time::sleep(",
                    "time::sleep_until(",
                ]
                .iter()
                .any(|marker| text.contains(marker))
                    && !matches!(&*await_expr.base, Expr::Call(call) if is_budgeted_call(call, false));
                self.raw_stream_wait |= raw_wait;
            }
            syn::visit::visit_expr_await(self, await_expr);
        }
    }
    let Ok(item) = syn::parse_str::<ItemFn>(source) else {
        return false;
    };
    let mut visitor = Find {
        source,
        in_spawn: false,
        spawn_state: SpawnState::None,
        raw_stream_wait: false,
    };
    syn::visit::Visit::visit_item_fn(&mut visitor, &item);
    matches!(visitor.spawn_state, SpawnState::Bounded) && !visitor.raw_stream_wait
}

pub(super) fn span_text(source: &str, span: Span) -> String {
    let start = offset(source, span.start().line, span.start().column);
    let end = offset(source, span.end().line, span.end().column);
    source.get(start..end).unwrap_or_default().to_owned()
}

fn offset(source: &str, line: usize, column: usize) -> usize {
    source
        .lines()
        .take(line.saturating_sub(1))
        .map(|value| value.len() + 1)
        .sum::<usize>()
        + column
}

//! Semantic Rust boundary checks that cannot be derived from Cargo metadata.

use super::Diagnostic;
use std::{collections::BTreeMap, ffi::OsStr, fs, path::Path};
use syn::{
    Expr, ExprCall, ExprField, ExprMacro, ExprMethodCall, ExprStruct, Ident, spanned::Spanned,
    visit::Visit,
};

const RULES: [&str; 8] = [
    "ARCH-ENV-ONLY-IN-CONFIG",
    "ARCH-HTTP-ONLY-IN-INTEGRATIONS",
    "ARCH-PROCESS-ONLY-IN-ADAPTERS",
    "ARCH-FILESYSTEM-ONLY-IN-ADAPTERS",
    "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT",
    "SEC-NO-SENSITIVE-LOG-ARGUMENTS",
    "SEC-SENTINEL-NO-PLAINTEXT",
    "ARCH-VM-PROVIDER-ONLY-IN-COMPOSITION",
];

pub(super) fn validate(root: &Path, enabled_rules: &[String], diagnostics: &mut Vec<Diagnostic>) {
    let active = RULES
        .into_iter()
        .filter(|rule| enabled_rules.iter().any(|enabled| enabled == rule))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return;
    }
    visit_sources(root, &root.join("crates"), &active, diagnostics);
    if active.contains(&"SEC-SENTINEL-NO-PLAINTEXT") {
        scan_repository_sentinels(root, diagnostics);
    }
}

pub(super) fn audit(root: &Path) -> BTreeMap<&'static str, usize> {
    let mut diagnostics = Vec::new();
    visit_sources(root, &root.join("crates"), &RULES, &mut diagnostics);
    scan_repository_sentinels(root, &mut diagnostics);
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
}

fn scan_repository_sentinels(root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    scan_sentinel_directory(root, root, diagnostics);
}

fn scan_sentinel_directory(root: &Path, directory: &Path, diagnostics: &mut Vec<Diagnostic>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if path.is_dir() {
            if should_skip_sentinel_directory(relative) {
                continue;
            }
            scan_sentinel_directory(root, &path, diagnostics);
        } else if is_scannable_source(&path) {
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            scan_sentinel_source(relative, &source, diagnostics);
        }
    }
}

fn should_skip_sentinel_directory(relative: &Path) -> bool {
    relative.starts_with("target")
        || relative.starts_with(".git")
        // `.local` contains private generated runtime and release state, not
        // repository source. Its evidence can legitimately quote scanner data.
        || relative.starts_with(".local")
        || relative.starts_with("web/deps")
        || relative.starts_with("web/_build")
        // The standalone released example vendors checksum-locked third-party
        // sources for offline builds. Their terminology/test data and generated
        // Cargo output are not application secret fixtures; scan its own src.
        || relative.starts_with("examples/cooking/cooking-gateway/vendor")
        || relative.starts_with("examples/cooking/cooking-gateway/target")
        || relative.starts_with("crates/heph-dev")
        || relative.starts_with("crates/heph-core/platform/rpc-proto/src/generated")
        || is_test_path(relative)
}

fn is_scannable_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("rs" | "ex" | "exs" | "proto" | "json")
    )
}

fn scan_sentinel_source(path: &Path, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    if path.to_string_lossy().contains("integration-check") {
        return;
    }
    let mut test_module = false;
    for (line_number, line) in source.lines().enumerate() {
        if line.contains("#[cfg(test)]") {
            test_module = true;
        }
        if !test_module && line.to_ascii_lowercase().contains("sentinel") {
            diagnostics.push(Diagnostic::new(
                "SEC-SENTINEL-NO-PLAINTEXT",
                format!(
                    "{}:{} contains a secret sentinel outside test-only or integration-check code",
                    path.display(),
                    line_number + 1
                ),
            ));
            break;
        }
    }
}

fn visit_sources(
    root: &Path,
    directory: &Path,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if path.is_dir() {
            if path.ends_with("target")
                || relative.starts_with("tests/fixtures")
                || relative.starts_with("crates/heph-dev")
                || relative.starts_with("crates/heph-core/platform/rpc-proto/src/generated")
            {
                continue;
            }
            visit_sources(root, &path, active, diagnostics);
        } else if path.extension() == Some(OsStr::new("rs")) {
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            validate_source(relative, &source, active, diagnostics);
        }
    }
}

// The semantic boundary rules share one source walk to keep path classification consistent.
#[allow(clippy::too_many_lines)]
fn validate_source(path: &Path, source: &str, active: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if is_test_path(path) {
        return;
    }
    let source = source.split("#[cfg(test)]").next().unwrap_or(source);
    if active.contains(&"ARCH-ENV-ONLY-IN-CONFIG")
        && contains_any(
            source,
            &[
                "std::env::var(",
                "std::env::var_os(",
                "std::env::args(",
                "std::env::args_os(",
            ],
        )
        && !is_configuration_path(path)
    {
        report(
            diagnostics,
            "ARCH-ENV-ONLY-IN-CONFIG",
            path,
            "reads process environment outside configuration/composition code",
        );
    }
    if active.contains(&"ARCH-HTTP-ONLY-IN-INTEGRATIONS")
        && contains_any(
            source,
            &[
                "reqwest::Client::new(",
                "hyper::Client::new(",
                "HttpClient::new(",
            ],
        )
        && !is_integration_path(path)
    {
        report(
            diagnostics,
            "ARCH-HTTP-ONLY-IN-INTEGRATIONS",
            path,
            "constructs an outbound HTTP client outside an integration adapter",
        );
    }
    if active.contains(&"ARCH-PROCESS-ONLY-IN-ADAPTERS")
        && contains_any(
            source,
            &[
                "std::process::Command::new(",
                "tokio::process::Command::new(",
                "process::Command::new(",
            ],
        )
        && !is_integration_path(path)
    {
        report(
            diagnostics,
            "ARCH-PROCESS-ONLY-IN-ADAPTERS",
            path,
            "constructs a host process outside an adapter or integration",
        );
    }
    if active.contains(&"ARCH-VM-PROVIDER-ONLY-IN-COMPOSITION")
        && contains_any(source, &["vm_fake::", "vm_libkrun::"])
        && !is_configuration_path(path)
        && !is_integration_path(path)
    {
        report(
            diagnostics,
            "ARCH-VM-PROVIDER-ONLY-IN-COMPOSITION",
            path,
            "imports a VM provider outside composition or an adapter",
        );
    }
    if active.contains(&"ARCH-FILESYSTEM-ONLY-IN-ADAPTERS")
        && contains_any(
            source,
            &[
                "std::fs::",
                "tokio::fs::",
                "fs::read(",
                "fs::write(",
                "OpenOptions::new(",
            ],
        )
        && !is_storage_path(path)
        // ARCH-FILESYSTEM-ONLY-IN-ADAPTERS: this build script reads migrations only to fingerprint
        // SQLx's compile-time embedding; it performs no runtime filesystem I/O.
        && !is_migration_fingerprint_build_script(path)
    {
        report(
            diagnostics,
            "ARCH-FILESYSTEM-ONLY-IN-ADAPTERS",
            path,
            "performs filesystem I/O outside a storage/runtime adapter",
        );
    }
    if active.contains(&"SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT") && contains_sensitive_format(source)
    {
        report(
            diagnostics,
            "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT",
            path,
            "formats a sensitive value with an unrestricted debug/display formatter",
        );
    }
    if active.contains(&"SEC-NO-SENSITIVE-LOG-ARGUMENTS") && contains_sensitive_log(source) {
        report(
            diagnostics,
            "SEC-NO-SENSITIVE-LOG-ARGUMENTS",
            path,
            "passes a sensitive value directly to a tracing/logging macro",
        );
    }
    if active.contains(&"SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT")
        || active.contains(&"SEC-NO-SENSITIVE-LOG-ARGUMENTS")
    {
        detect_sensitive_flows(path, source, active, diagnostics);
    }
}

#[derive(Clone, Copy)]
enum SensitiveValue {
    Plain {
        origin: &'static str,
        source_line: usize,
    },
    Opaque,
}

fn detect_sensitive_flows(
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
            Expr::Path(path) => path
                .path
                .get_ident()
                .and_then(|name| self.bindings.get(&name.to_string()).copied()),
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
        if is_sensitive_request_field(&field_name) && is_request_expr(&field.base) {
            return Some(SensitiveValue::Plain {
                origin: "sensitive request field",
                source_line: span_line(field),
            });
        }
        self.sensitive_expr(&field.base)
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
                SensitiveValue::Opaque => None,
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
        SensitiveValue::Opaque => None,
    }
}

fn span_line<T: Spanned>(node: &T) -> usize {
    node.span().start().line
}

fn is_sensitive_request_field(field: &str) -> bool {
    sensitive_request_field_names().contains(&field)
}

const fn sensitive_request_field_names() -> [&'static str; 8] {
    [
        "handoff_secret",
        "sid",
        "value",
        "token",
        "credential",
        "password",
        "plaintext",
        "ciphertext",
    ]
}

fn is_request_expr(expression: &Expr) -> bool {
    match expression {
        Expr::Path(path) => path
            .path
            .get_ident()
            .is_some_and(|ident| ident == "request"),
        Expr::Reference(reference) => is_request_expr(&reference.expr),
        Expr::Paren(parenthesized) => is_request_expr(&parenthesized.expr),
        _ => false,
    }
}

fn is_opaque_conversion(function: &Expr) -> bool {
    let names = expression_path_names(function);
    names.ends_with("Uuid::from_slice")
        || names.ends_with("OpaqueId::from_uuid")
        || names.ends_with("OpaqueId::from_bytes")
}

fn macro_sink_rule(_path: &syn::Path, name: &str) -> &'static str {
    if matches!(
        name,
        "format" | "format_args" | "debug" | "display" | "anyhow" | "bail"
    ) {
        "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT"
    } else {
        "SEC-NO-SENSITIVE-LOG-ARGUMENTS"
    }
}

fn is_supported_sensitive_macro(path: &syn::Path, name: &str) -> bool {
    let root = path
        .segments
        .first()
        .map(|segment| segment.ident.to_string());
    matches!(
        name,
        "format"
            | "format_args"
            | "debug"
            | "display"
            | "anyhow"
            | "bail"
            | "ensure"
            | "json"
            | "append_application_event"
            | "publish_event"
            | "emit_event"
            | "info"
            | "warn"
            | "error"
            | "trace"
    ) || matches!(root.as_deref(), Some("tracing" | "log" | "serde_json"))
}

fn call_sink_rule(function: &Expr) -> Option<&'static str> {
    let text = expression_path_names(function);
    let last = text.rsplit("::").next().unwrap_or_default();
    if (text.contains("Response") || text.contains("Event") || text.contains("Payload"))
        || text.contains("Error")
        || matches!(last, "to_value" | "to_json" | "to_string")
        || matches!(
            last,
            "append_application_event" | "publish_event" | "emit_event"
        )
    {
        Some("SEC-NO-SENSITIVE-LOG-ARGUMENTS")
    } else {
        None
    }
}

fn method_sink_rule(method: &Ident, receiver: &Expr) -> Option<&'static str> {
    let method = method.to_string();
    if matches!(
        method.as_str(),
        "with_label_values" | "label_values" | "label" | "with_label"
    ) || (method == "insert" && receiver_mentions_label(receiver))
        || matches!(
            method.as_str(),
            "set_value" | "field" | "body" | "json" | "append" | "publish" | "emit"
        )
    {
        Some("SEC-NO-SENSITIVE-LOG-ARGUMENTS")
    } else if method == "to_string" {
        Some("SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT")
    } else {
        None
    }
}

fn receiver_mentions_label(receiver: &Expr) -> bool {
    expression_path_names(receiver)
        .to_ascii_lowercase()
        .contains("label")
}

fn expression_path_names(expression: &Expr) -> String {
    match expression {
        Expr::Path(path) => path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        Expr::Field(field) => {
            let mut names = expression_path_names(&field.base);
            if let syn::Member::Named(member) = &field.member {
                if !names.is_empty() {
                    names.push_str("::");
                }
                names.push_str(&member.to_string());
            }
            names
        }
        Expr::Reference(reference) => expression_path_names(&reference.expr),
        Expr::Paren(parenthesized) => expression_path_names(&parenthesized.expr),
        _ => String::new(),
    }
}

fn is_sensitive_output_type(ident: &Ident) -> bool {
    let name = ident.to_string();
    name.contains("Response") || name.contains("Event") || name.contains("Payload")
}

fn token_contains_name(tokens: &str, name: &str) -> bool {
    let normalized = tokens
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    normalized
        .split_whitespace()
        .any(|token| token == name || token == name.replace(' ', ""))
        || tokens.replace(' ', "").contains(&name.replace(' ', ""))
}

fn report(diagnostics: &mut Vec<Diagnostic>, rule: &'static str, path: &Path, message: &str) {
    diagnostics.push(Diagnostic::new(
        rule,
        format!("{} {message}", path.display()),
    ));
}

fn contains_any(source: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| source.contains(needle))
}

fn contains_sensitive_format(source: &str) -> bool {
    source.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        (lower.contains("format!(") || lower.contains("debug!(") || lower.contains("display!("))
            && contains_sensitive_binding(&lower)
            && (lower.contains("{:?") || lower.contains("{:#?") || lower.contains("{}"))
    })
}

fn contains_sensitive_log(source: &str) -> bool {
    source.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        (lower.contains("tracing::") || lower.contains("log::"))
            && contains_sensitive_binding(&lower)
            && contains_any(&lower, &["?", "%", "format!"])
    })
}

fn contains_sensitive_binding(source: &str) -> bool {
    contains_any(
        source,
        &[
            "secret)",
            "secret,",
            "secret.",
            "credential)",
            "credential,",
            "credential.",
            "token)",
            "token,",
            "token.",
            "password)",
            "password,",
            "password.",
            "plaintext)",
            "plaintext,",
            "ciphertext)",
            "ciphertext,",
        ],
    )
}

fn is_test_path(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component.as_os_str().to_str(), Some("test" | "tests")))
}

fn is_configuration_path(path: &Path) -> bool {
    path.starts_with("crates/heph-app")
        || path.starts_with("crates/heph-std/runtime/vm/libkrun")
        || path.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("config" | "configuration" | "composition" | "bootstrap" | "bin")
            )
        })
        || path.file_name().is_some_and(|name| name == "main.rs")
}

fn is_integration_path(path: &Path) -> bool {
    path.starts_with("crates/heph-std/runtime/vm/libkrun")
        || path.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some(
                    "adapter"
                        | "adapters"
                        | "integration"
                        | "integrations"
                        | "http"
                        | "git_http"
                        | "bootstrap"
                        | "bin"
                        | "review-service"
                )
            )
        })
}

fn is_storage_path(path: &Path) -> bool {
    is_integration_path(path)
        || [
            "crates/heph-app",
            "crates/heph-core/forge/build/orchestrator",
            "crates/heph-core/forge/release/artifact-store",
            "crates/heph-core/forge/service",
            "crates/heph-core/identity/git-credential",
            "crates/heph-dev/vm/conformance",
            "crates/heph-std/authorization/runtime-handoff-local",
            "crates/heph-std/forge/build/oci-builder-runtime-local",
            "crates/heph-std/forge/build/oci-builder-worker",
            "crates/heph-std/forge/registry/publisher",
            "crates/heph-std/run/runtime-local",
            "crates/heph-std/runtime/vm/fake",
            "crates/heph-std/runtime/volume/local",
            "crates/heph-std/runtime/workspace/local",
            "crates/heph-std/secret/runtime",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
        || path.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("artifact" | "repository" | "workspace" | "volume" | "runtime" | "store")
            )
        })
}

fn is_migration_fingerprint_build_script(path: &Path) -> bool {
    path == Path::new("crates/heph-core/authorization/runtime-authority-postgres/build.rs")
}

#[cfg(test)]
mod tests {
    use super::{RULES, is_configuration_path, is_storage_path, validate_source};
    use std::path::Path;

    fn all_rules() -> Vec<&'static str> {
        RULES.to_vec()
    }

    #[test]
    fn valid_configuration_and_adapter_fixture_is_accepted() {
        let active = all_rules();
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/config/settings.rs"),
            include_str!("../../../tests/fixtures/rust-architecture/valid/src/config/settings.rs"),
            &active,
            &mut diagnostics,
        );
        validate_source(
            Path::new("crates/example/src/http/client.rs"),
            include_str!("../../../tests/fixtures/rust-architecture/valid/src/http/client.rs"),
            &active,
            &mut diagnostics,
        );
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
    }

    #[test]
    fn sensitive_flow_fixture_allows_opaque_conversions() {
        let active = all_rules();
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/rpc/sensitive.rs"),
            include_str!("../../../tests/fixtures/rust-architecture/valid/src/sensitive_flow.rs"),
            &active,
            &mut diagnostics,
        );
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
    }

    #[test]
    fn sensitive_flow_fixture_reports_source_and_sink_without_plaintext() {
        let active = all_rules();
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/rpc/sensitive.rs"),
            include_str!("../../../tests/fixtures/rust-architecture/invalid/src/sensitive_flow.rs"),
            &active,
            &mut diagnostics,
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == "SEC-NO-SENSITIVE-LOG-ARGUMENTS")
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT")
        );
        let flow_diagnostics = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.contains("sensitive request field"))
            .collect::<Vec<_>>();
        assert!(flow_diagnostics.len() >= 7, "diagnostics: {diagnostics:?}");
        assert!(
            flow_diagnostics
                .iter()
                .all(|diagnostic| diagnostic.message.contains("sink"))
        );
        assert!(flow_diagnostics.iter().all(|diagnostic| {
            diagnostic.message.contains("source at line ")
                && diagnostic.message.contains("in rejected")
                && diagnostic.message.contains("line ")
        }));
        assert!(flow_diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("info sink")
                && diagnostic.message.contains("source at line 3")
                && diagnostic.message.contains("line 4")
        }));
        assert!(flow_diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("format sink")
                && diagnostic.message.contains("source at line 3")
                && diagnostic.message.contains("line 5")
        }));
        for sink in [
            "info",
            "format",
            "anyhow",
            "json",
            "insert",
            "label",
            "publish",
            "body",
            "to_string",
        ] {
            assert!(
                flow_diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(sink)),
                "missing {sink} diagnostic: {diagnostics:?}"
            );
        }
        assert!(diagnostics.iter().all(|diagnostic| {
            !diagnostic.message.contains("received handoff")
                && !diagnostic.message.contains("invalid handoff")
        }));
    }

    #[test]
    fn migration_fingerprint_exception_is_exactly_scoped_to_its_build_script() {
        let active = all_rules();
        let source = "use std::fs; fn main() { let _ = fs::read_dir(\"migrations\"); let _ = fs::read(\"migration.sql\"); }";

        let mut build_script_diagnostics = Vec::new();
        validate_source(
            Path::new("crates/heph-core/authorization/runtime-authority-postgres/build.rs"),
            source,
            &active,
            &mut build_script_diagnostics,
        );
        assert!(build_script_diagnostics.is_empty());

        let mut neighboring_diagnostics = Vec::new();
        validate_source(
            Path::new("crates/other-adapter/build.rs"),
            source,
            &active,
            &mut neighboring_diagnostics,
        );
        assert!(
            neighboring_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == "ARCH-FILESYSTEM-ONLY-IN-ADAPTERS")
        );
    }

    #[test]
    fn moved_topology_paths_keep_exact_boundary_classification() {
        assert!(is_configuration_path(Path::new(
            "crates/heph-std/runtime/vm/libkrun/src/worker.rs"
        )));
        for path in [
            "crates/heph-core/forge/build/orchestrator/src/lib.rs",
            "crates/heph-core/forge/release/artifact-store/src/lib.rs",
            "crates/heph-core/forge/service/src/storage.rs",
            "crates/heph-core/identity/git-credential/src/main.rs",
            "crates/heph-std/forge/registry/publisher/src/lib.rs",
            "crates/heph-std/run/runtime-local/src/lib.rs",
        ] {
            assert!(is_storage_path(Path::new(path)), "{path}");
        }
        assert!(!is_storage_path(Path::new(
            "crates/heph-core/forge/build/unrelated/src/lib.rs"
        )));
    }

    #[test]
    fn invalid_fixture_reports_actionable_boundary_diagnostics() {
        let active = all_rules();
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/application.rs"),
            include_str!("../../../tests/fixtures/rust-architecture/invalid/src/application.rs"),
            &active,
            &mut diagnostics,
        );
        for rule in RULES
            .iter()
            .copied()
            .filter(|rule| *rule != "SEC-SENTINEL-NO-PLAINTEXT")
        {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule),
                "missing diagnostic for {rule}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn sentinel_scan_allows_test_only_values_and_rejects_production_values() {
        let mut valid = Vec::new();
        super::scan_sentinel_source(
            Path::new("crates/example/src/lib.rs"),
            include_str!("../../../tests/fixtures/secret-safety/valid/src/lib.rs"),
            &mut valid,
        );
        assert!(valid.is_empty(), "unexpected diagnostics: {valid:?}");

        let mut invalid = Vec::new();
        super::scan_sentinel_source(
            Path::new("crates/example/src/lib.rs"),
            include_str!("../../../tests/fixtures/secret-safety/invalid/src/lib.rs"),
            &mut invalid,
        );
        assert_eq!(invalid.len(), 1);
        assert_eq!(invalid[0].rule_id, "SEC-SENTINEL-NO-PLAINTEXT");
    }

    #[test]
    fn sentinel_scan_skips_private_local_state() {
        assert!(super::should_skip_sentinel_directory(Path::new(".local")));
        assert!(!super::should_skip_sentinel_directory(Path::new(
            "crates/example"
        )));
    }

    #[test]
    fn example_vendor_exclusion_keeps_application_sources_scanned() {
        for path in [
            "examples/cooking/cooking-gateway/vendor/memchr/src",
            "examples/cooking/cooking-gateway/target/debug",
        ] {
            assert!(super::should_skip_sentinel_directory(Path::new(path)));
        }
        let application = Path::new("examples/cooking/cooking-gateway/src/main.rs");
        assert!(!super::should_skip_sentinel_directory(application));
        assert!(!super::should_skip_sentinel_directory(Path::new(
            "examples/cooking/cooking-gateway/vendor-like"
        )));
        let mut diagnostics = Vec::new();
        super::scan_sentinel_source(
            application,
            "const LEAK: &str = \"secret-sentinel\";",
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 1);
    }
}

//! Rule predicates and path classification.

use super::Diagnostic;
use std::path::Path;
use syn::{Expr, Ident};

pub(super) fn is_sensitive_request_field(field: &str) -> bool {
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

pub(super) fn is_opaque_conversion(function: &Expr) -> bool {
    let names = expression_path_names(function);
    names.ends_with("Uuid::from_slice")
        || names.ends_with("OpaqueId::from_uuid")
        || names.ends_with("OpaqueId::from_bytes")
}

pub(super) fn macro_sink_rule(_path: &syn::Path, name: &str) -> &'static str {
    if matches!(
        name,
        "format" | "format_args" | "debug" | "display" | "anyhow" | "bail"
    ) {
        "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT"
    } else {
        "SEC-NO-SENSITIVE-LOG-ARGUMENTS"
    }
}

pub(super) fn is_supported_sensitive_macro(path: &syn::Path, name: &str) -> bool {
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

pub(super) fn call_sink_rule(function: &Expr) -> Option<&'static str> {
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

pub(super) fn method_sink_rule(method: &Ident, receiver: &Expr) -> Option<&'static str> {
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

pub(super) fn expression_path_names(expression: &Expr) -> String {
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

pub(super) fn is_sensitive_output_type(ident: &Ident) -> bool {
    let name = ident.to_string();
    name.contains("Response") || name.contains("Event") || name.contains("Payload")
}

pub(super) fn token_contains_name(tokens: &str, name: &str) -> bool {
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

pub(super) fn report(
    diagnostics: &mut Vec<Diagnostic>,
    rule: &'static str,
    path: &Path,
    message: &str,
) {
    diagnostics.push(Diagnostic::new(
        rule,
        format!("{} {message}", path.display()),
    ));
}

pub(super) fn contains_any(source: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| source.contains(needle))
}

pub(super) fn contains_sensitive_format(source: &str) -> bool {
    source.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        (lower.contains("format!(") || lower.contains("debug!(") || lower.contains("display!("))
            && contains_sensitive_binding(&lower)
            && (lower.contains("{:?") || lower.contains("{:#?") || lower.contains("{}"))
    })
}

pub(super) fn contains_sensitive_log(source: &str) -> bool {
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

pub(super) fn is_test_path(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component.as_os_str().to_str(), Some("test" | "tests")))
}

pub(super) fn is_configuration_path(path: &Path) -> bool {
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

pub(super) fn is_integration_path(path: &Path) -> bool {
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

pub(super) fn is_storage_path(path: &Path) -> bool {
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

pub(super) fn is_migration_fingerprint_build_script(path: &Path) -> bool {
    path == Path::new("crates/heph-core/authorization/runtime-authority-postgres/build.rs")
}

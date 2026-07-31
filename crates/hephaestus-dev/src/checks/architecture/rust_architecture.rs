//! Semantic Rust boundary checks that cannot be derived from Cargo metadata.

use super::Diagnostic;
use std::{collections::BTreeMap, ffi::OsStr, fs, path::Path};

const RULES: [&str; 7] = [
    "ARCH-ENV-ONLY-IN-CONFIG",
    "ARCH-HTTP-ONLY-IN-INTEGRATIONS",
    "ARCH-PROCESS-ONLY-IN-ADAPTERS",
    "ARCH-FILESYSTEM-ONLY-IN-ADAPTERS",
    "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT",
    "SEC-NO-SENSITIVE-LOG-ARGUMENTS",
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
}

pub(super) fn audit(root: &Path) -> BTreeMap<&'static str, usize> {
    let mut diagnostics = Vec::new();
    visit_sources(root, &root.join("crates"), &RULES, &mut diagnostics);
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
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
                || relative.starts_with("crates/hephaestus-dev")
                || relative.starts_with("crates/rpc-proto/src/generated")
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
        .any(|component| component.as_os_str() == OsStr::new("tests"))
}

fn is_configuration_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(
                "config"
                    | "configuration"
                    | "composition"
                    | "bootstrap"
                    | "bin"
                    | "vm-libkrun"
                    | "hephaestus-app",
            )
        )
    }) || path.file_name().is_some_and(|name| name == "main.rs")
}

fn is_integration_path(path: &Path) -> bool {
    path.components().any(|component| {
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
                    | "vm-libkrun"
            )
        )
    })
}

fn is_storage_path(path: &Path) -> bool {
    is_integration_path(path)
        || path.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some(
                    "artifact"
                        | "repository"
                        | "workspace"
                        | "volume"
                        | "runtime"
                        | "store"
                        | "hephaestus-app"
                        | "vm-fake"
                        | "release-artifact-store"
                        | "workspace-local"
                        | "run-runtime-local"
                        | "secret-runtime"
                        | "forge-service"
                        | "vm-conformance"
                        | "volume-local"
                        | "build-orchestrator"
                )
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{RULES, validate_source};
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
    fn invalid_fixture_reports_actionable_boundary_diagnostics() {
        let active = all_rules();
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/application.rs"),
            include_str!("../../../tests/fixtures/rust-architecture/invalid/src/application.rs"),
            &active,
            &mut diagnostics,
        );
        for rule in RULES {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule),
                "missing diagnostic for {rule}: {diagnostics:?}"
            );
        }
    }
}

//! Semantic Rust boundary checks that cannot be derived from Cargo metadata.

use super::Diagnostic;
use std::{collections::BTreeMap, ffi::OsStr, fs, path::Path};

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
        || relative.starts_with("crates/hephaestus-dev")
        || relative.starts_with("crates/rpc-proto/src/generated")
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
        .any(|component| matches!(component.as_os_str().to_str(), Some("test" | "tests")))
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
                        | "registry-publisher"
                        | "registry-release"
                        | "workspace-local"
                        | "run-runtime-local"
                        | "oci-builder-worker"
                        | "oci-builder-runtime-local"
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
}

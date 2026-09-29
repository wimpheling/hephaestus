//! Source walking and semantic boundary validation.

use super::{
    Diagnostic,
    rules::{
        contains_any, contains_sensitive_format, contains_sensitive_log, is_configuration_path,
        is_integration_path, is_migration_fingerprint_build_script, is_storage_path, is_test_path,
        report,
    },
    sensitive_flow::detect_sensitive_flows,
};
use std::{ffi::OsStr, fs, path::Path};

pub(super) fn scan_repository_sentinels(root: &Path, diagnostics: &mut Vec<Diagnostic>) {
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

pub(super) fn should_skip_sentinel_directory(relative: &Path) -> bool {
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

pub(super) fn scan_sentinel_source(path: &Path, source: &str, diagnostics: &mut Vec<Diagnostic>) {
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

pub(super) fn visit_sources(
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
pub(super) fn validate_source(
    path: &Path,
    source: &str,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
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

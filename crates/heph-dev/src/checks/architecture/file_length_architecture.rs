//! Repository-wide physical line limits for hand-maintained Rust sources.

use super::Diagnostic;
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

pub(super) const RULE: &str = "ARCH-MAX-FILE-LENGTH";
const RUST_LIMIT: usize = 350;
const RUST_THRESHOLD_KEYS: [&str; 6] = [
    "domain",
    "application",
    "adapter",
    "transport",
    "composition",
    "development",
];
const GENERATED_RPC: &str = "crates/heph-core/platform/rpc-proto/src/generated";
const VENDORED_COOKING: &str = "examples/cooking/cooking-gateway/vendor";

pub(super) fn validate(
    root: &Path,
    enabled_rules: &[String],
    maximum_file_lines: &BTreeMap<String, usize>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !enabled_rules.iter().any(|rule| rule == RULE) {
        return;
    }
    let Some(limit) = configured_rust_limit(maximum_file_lines, diagnostics) else {
        return;
    };
    let result = scan_with_errors(root, limit);
    for error in result.errors {
        diagnostics.push(Diagnostic::new(
            RULE,
            format!(
                "{} could not be read while enforcing the {limit}-line limit: {}; restore repository source readability",
                error.path.display(),
                error.error
            ),
        ));
    }
    for (path, line_count) in result.findings {
        diagnostics.push(Diagnostic::new(
            RULE,
            format!(
                "{} has {line_count} physical lines; maximum is {limit}; split cohesive modules or test responsibilities while preserving public paths",
                path.display()
            ),
        ));
    }
}

pub(super) fn audit(
    root: &Path,
    maximum_file_lines: &BTreeMap<String, usize>,
) -> BTreeMap<&'static str, usize> {
    let Some(limit) = RUST_THRESHOLD_KEYS
        .iter()
        .find_map(|key| maximum_file_lines.get(*key).copied())
    else {
        return BTreeMap::new();
    };
    let count = scan(root, limit).len();
    (count > 0).then_some((RULE, count)).into_iter().collect()
}

fn configured_rust_limit(
    maximum_file_lines: &BTreeMap<String, usize>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<usize> {
    let mut configured = Vec::new();
    for key in RUST_THRESHOLD_KEYS {
        match maximum_file_lines.get(key).copied() {
            Some(limit) => configured.push((key, limit)),
            None => diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("{RULE} requires a configured Rust threshold named `{key}`"),
            )),
        }
    }
    if configured.iter().any(|(_, limit)| *limit != RUST_LIMIT) {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{RULE} requires every Rust threshold to be exactly {RUST_LIMIT} lines"),
        ));
    }
    configured.first().map(|(_, limit)| *limit)
}

fn scan(root: &Path, limit: usize) -> Vec<(PathBuf, usize)> {
    scan_with_errors(root, limit).findings
}

struct ScanResult {
    findings: Vec<(PathBuf, usize)>,
    errors: Vec<ScanError>,
}

struct ScanError {
    path: PathBuf,
    error: String,
}

fn scan_with_errors(root: &Path, limit: usize) -> ScanResult {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    for directory in [root.join("crates"), root.join("examples")] {
        collect_rust_files(root, &canonical_root, &directory, &mut files, &mut errors);
    }
    files.sort();
    let mut findings = files
        .into_iter()
        .filter_map(|path| match fs::read_to_string(&path) {
            Ok(source) => {
                let line_count = source.lines().count();
                (line_count > limit).then_some((path, line_count))
            }
            Err(error) => {
                errors.push(ScanError {
                    path,
                    error: error.to_string(),
                });
                None
            }
        })
        .map(|(path, line_count)| {
            (
                path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
                line_count,
            )
        })
        .collect::<Vec<_>>();
    findings.sort();
    errors.sort_by(|left, right| left.path.cmp(&right.path));
    ScanResult { findings, errors }
}

fn collect_rust_files(
    root: &Path,
    canonical_root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
    errors: &mut Vec<ScanError>,
) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(ScanError {
                path: directory.to_path_buf(),
                error: error.to_string(),
            });
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(ScanError {
                    path: directory.to_path_buf(),
                    error: error.to_string(),
                });
                continue;
            }
        };
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(ScanError {
                    path,
                    error: error.to_string(),
                });
                continue;
            }
        };
        if is_excluded(relative) || is_cargo_target(relative) {
            continue;
        }
        if file_type.is_symlink() {
            if path.extension() != Some(OsStr::new("rs")) {
                continue;
            }
            let target = match fs::canonicalize(&path) {
                Ok(target) => target,
                Err(error) => {
                    errors.push(ScanError {
                        path,
                        error: format!("broken or unreadable Rust symlink: {error}"),
                    });
                    continue;
                }
            };
            if !target.starts_with(canonical_root) {
                errors.push(ScanError {
                    path,
                    error: format!(
                        "Rust symlink target escapes repository: {}",
                        target.display()
                    ),
                });
                continue;
            }
            let target_relative = target.strip_prefix(canonical_root).unwrap_or(&target);
            if is_excluded(target_relative) || is_cargo_target(target_relative) {
                continue;
            }
            match fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => files.push(path),
                Ok(_) => errors.push(ScanError {
                    path,
                    error: String::from("Rust symlink target is not a regular file"),
                }),
                Err(error) => errors.push(ScanError {
                    path,
                    error: error.to_string(),
                }),
            }
            continue;
        }
        if file_type.is_dir() {
            collect_rust_files(root, canonical_root, &path, files, errors);
        } else if file_type.is_file() && path.extension() == Some(OsStr::new("rs")) {
            files.push(path);
        }
    }
}

fn is_excluded(relative: &Path) -> bool {
    relative.starts_with(Path::new(GENERATED_RPC))
        || relative.starts_with(Path::new(VENDORED_COOKING))
}

fn is_cargo_target(relative: &Path) -> bool {
    relative
        .components()
        .any(|component| component.as_os_str() == OsStr::new("target"))
}

#[cfg(test)]
mod tests;

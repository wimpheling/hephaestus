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
mod tests {
    use super::{RULE, RUST_THRESHOLD_KEYS, scan, validate};
    use std::{collections::BTreeMap, fs, path::Path};
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    fn write_lines(root: &Path, relative: &str, count: usize) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
        fs::create_dir_all(root.join("crates")).expect("crates source root");
        fs::create_dir_all(root.join("examples")).expect("examples source root");
        let source = (0..count)
            .map(|line| match line {
                0 => String::from("// comment counts"),
                1 => String::new(),
                2 => String::from("#[cfg(test)]"),
                _ => String::from("fn fixture_line() {}"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, source).expect("fixture source");
    }

    #[test]
    fn accepts_exact_limit_and_counts_blank_comment_and_test_lines() {
        let directory = tempdir().expect("fixture root");
        write_lines(directory.path(), "crates/context/src/lib.rs", 350);

        assert!(scan(directory.path(), 350).is_empty());
    }

    #[test]
    fn rejects_one_line_over_limit_with_nested_build_example_and_untracked_paths() {
        let directory = tempdir().expect("fixture root");
        write_lines(directory.path(), "crates/context/src/nested/module.rs", 351);
        write_lines(directory.path(), "crates/context/build.rs", 351);
        write_lines(directory.path(), "crates/context/tests/integration.rs", 351);
        write_lines(directory.path(), "examples/example/src/main.rs", 351);

        let findings = scan(directory.path(), 350);
        assert_eq!(findings.len(), 4);
        assert!(findings.iter().all(|(_, count)| *count == 351));
        assert!(findings.iter().all(|(path, _)| path.extension().is_some()));
        assert_eq!(RULE, "ARCH-MAX-FILE-LENGTH");

        let mut diagnostics = Vec::new();
        let enabled = vec![RULE.to_owned()];
        let thresholds = RUST_THRESHOLD_KEYS
            .into_iter()
            .map(|key| (key.to_owned(), 350))
            .collect::<BTreeMap<_, _>>();
        validate(directory.path(), &enabled, &thresholds, &mut diagnostics);
        assert_eq!(diagnostics.len(), 4);
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.message.contains("351 physical lines")
                && diagnostic.message.contains("maximum is 350")
                && diagnostic.message.contains("split cohesive modules")
        }));
    }

    #[test]
    fn excludes_only_the_named_generated_and_vendored_subtrees() {
        let directory = tempdir().expect("fixture root");
        write_lines(
            directory.path(),
            "crates/heph-core/platform/rpc-proto/src/generated/connect.rs",
            351,
        );
        write_lines(
            directory.path(),
            "examples/cooking/cooking-gateway/vendor/serde/src/lib.rs",
            351,
        );
        write_lines(directory.path(), "crates/context/generated/module.rs", 351);
        write_lines(directory.path(), "examples/context/vendor/module.rs", 351);

        let findings = scan(directory.path(), 350);
        assert_eq!(findings.len(), 2);
        assert_eq!(
            findings
                .iter()
                .map(|(path, count)| (path.to_string_lossy().into_owned(), *count))
                .collect::<Vec<_>>(),
            vec![
                (String::from("crates/context/generated/module.rs"), 351),
                (String::from("examples/context/vendor/module.rs"), 351),
            ]
        );
        assert!(
            findings
                .iter()
                .any(|(path, _)| path.to_string_lossy().contains("crates/context/generated"))
        );
        assert!(
            findings
                .iter()
                .any(|(path, _)| path.to_string_lossy().contains("examples/context/vendor"))
        );
    }

    #[test]
    fn excludes_nested_cargo_target_build_output_without_excluding_source_neighbors() {
        let directory = tempdir().expect("fixture root");
        write_lines(
            directory.path(),
            "crates/context/target/debug/generated.rs",
            351,
        );
        write_lines(
            directory.path(),
            "examples/context/target/debug/build.rs",
            351,
        );
        write_lines(directory.path(), "crates/context/src/lib.rs", 351);

        let findings = scan(directory.path(), 350);
        assert_eq!(findings, vec![("crates/context/src/lib.rs".into(), 351)]);
    }

    #[test]
    fn enabled_rule_reports_source_root_read_errors() {
        let directory = tempdir().expect("fixture root");
        fs::create_dir_all(directory.path().join("examples")).expect("examples source root");
        fs::write(directory.path().join("crates"), "not a directory")
            .expect("blocking source root");
        let thresholds = RUST_THRESHOLD_KEYS
            .into_iter()
            .map(|key| (key.to_owned(), 350))
            .collect::<BTreeMap<_, _>>();
        let mut diagnostics = Vec::new();

        validate(
            directory.path(),
            &[RULE.to_owned()],
            &thresholds,
            &mut diagnostics,
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("crates"));
        assert!(diagnostics[0].message.contains("could not be read"));
        assert!(diagnostics[0].message.contains("Not a directory"));
    }

    #[cfg(unix)]
    #[test]
    fn follows_in_repository_rust_symlinks_and_scans_them_as_source() {
        let directory = tempdir().expect("fixture root");
        write_lines(directory.path(), "crates/context/src/real.rs", 351);
        symlink(
            directory.path().join("crates/context/src/real.rs"),
            directory.path().join("crates/context/src/linked.rs"),
        )
        .expect("source symlink");

        let findings = scan(directory.path(), 350);
        assert!(findings.iter().any(|(path, count)| {
            path == Path::new("crates/context/src/linked.rs") && *count == 351
        }));
    }

    #[cfg(unix)]
    #[test]
    fn enabled_rule_reports_rust_symlinks_that_escape_the_repository() {
        let directory = tempdir().expect("fixture root");
        let outside = tempdir().expect("outside fixture root");
        write_lines(outside.path(), "crates/outside.rs", 351);
        fs::create_dir_all(directory.path().join("crates/context/src"))
            .expect("fixture source directory");
        fs::create_dir_all(directory.path().join("examples")).expect("examples source root");
        symlink(
            outside.path().join("crates/outside.rs"),
            directory.path().join("crates/context/src/escaped.rs"),
        )
        .expect("escaping source symlink");
        let thresholds = RUST_THRESHOLD_KEYS
            .into_iter()
            .map(|key| (key.to_owned(), 350))
            .collect::<BTreeMap<_, _>>();
        let mut diagnostics = Vec::new();

        validate(
            directory.path(),
            &[RULE.to_owned()],
            &thresholds,
            &mut diagnostics,
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("escapes repository"));
    }
}

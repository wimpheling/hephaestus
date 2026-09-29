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
    fs::write(directory.path().join("crates"), "not a directory").expect("blocking source root");
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

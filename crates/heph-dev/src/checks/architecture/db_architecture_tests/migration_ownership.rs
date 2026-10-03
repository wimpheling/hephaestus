use std::{collections::BTreeSet, fs, path::Path};

use tempfile::tempdir;

use super::super::{MIGRATION_RULE, validate_rust_source};
use crate::checks::architecture::Diagnostic;

fn scan(root: &Path, include: &str) -> Vec<Diagnostic> {
    let relative = Path::new("src/adapters/lib.rs");
    let source = root.join(relative);
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(
        &source,
        format!("fn fixture() {{ sqlx::query(include_str!({include:?})); }}"),
    )
    .unwrap();
    let mut diagnostics = Vec::new();
    validate_rust_source(
        root,
        relative,
        &source,
        &BTreeSet::from([MIGRATION_RULE]),
        &[],
        &mut diagnostics,
    );
    diagnostics
}

fn write_schema(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "CREATE TABLE fixture (id bigint);").unwrap();
}

#[test]
fn portable_relative_include_is_owned_by_actual_root_migrations() {
    let root = tempdir().unwrap();
    write_schema(&root.path().join("migrations/test_fixtures/birth.sql"));
    assert!(scan(root.path(), "../../migrations/test_fixtures/birth.sql").is_empty());
}

#[test]
fn outside_and_nonexistent_includes_remain_denied() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("migrations")).unwrap();
    write_schema(&root.path().join("outside.sql"));
    for include in [
        "../../migrations/../outside.sql",
        "../../migrations/missing.sql",
    ] {
        let diagnostics = scan(root.path(), include);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule_id, MIGRATION_RULE);
    }
}

#[cfg(unix)]
#[test]
fn migration_file_symlink_cannot_escape_root_ownership() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let external = tempdir().unwrap();
    let outside = external.path().join("outside.sql");
    write_schema(&outside);
    fs::create_dir(root.path().join("migrations")).unwrap();
    symlink(outside, root.path().join("migrations/escape.sql")).unwrap();
    let diagnostics = scan(root.path(), "../../migrations/escape.sql");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule_id, MIGRATION_RULE);
}

#[cfg(unix)]
#[test]
fn external_migrations_directory_is_not_repository_owned() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let external = tempdir().unwrap();
    write_schema(&external.path().join("birth.sql"));
    symlink(external.path(), root.path().join("migrations")).unwrap();
    let diagnostics = scan(root.path(), "../../migrations/birth.sql");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule_id, MIGRATION_RULE);
}

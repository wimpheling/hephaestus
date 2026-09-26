use super::super::{
    PAGINATION_RULE, STATIC_RULE, validate_exception_scope, validate_metadata, validate_rust_source,
};
use crate::checks::architecture::{
    ArchitectureException, CargoDependency, CargoMetadata, CargoPackage, Diagnostic,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use tempfile::tempdir;

pub(super) fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/db-architecture")
        .join(name)
}

pub(super) fn package(
    root: &Path,
    name: &str,
    metadata: serde_json::Value,
    dependencies: Vec<CargoDependency>,
) -> CargoPackage {
    CargoPackage {
        id: format!("{name} 0.1.0"),
        name: name.to_owned(),
        manifest_path: root.join(name).join("Cargo.toml"),
        metadata,
        dependencies,
    }
}

pub(super) fn path_dependency(name: &str, path: PathBuf) -> CargoDependency {
    CargoDependency {
        name: name.to_owned(),
        path: Some(path),
        kind: None,
    }
}

pub(super) fn sqlx_dependency() -> CargoDependency {
    CargoDependency {
        name: String::from("sqlx"),
        path: None,
        kind: None,
    }
}

pub(super) fn dev_sqlx_dependency() -> CargoDependency {
    CargoDependency {
        name: String::from("sqlx"),
        path: None,
        kind: Some(String::from("dev")),
    }
}

pub(super) fn sqlx_metadata_diagnostics(metadata: &CargoMetadata) -> Vec<Diagnostic> {
    let active = BTreeSet::from([super::super::SQLX_RULE]);
    let mut diagnostics = Vec::new();
    validate_metadata(metadata, &active, &mut diagnostics);
    diagnostics
}

pub(super) fn static_exception(scope: &str, rule_id: &str) -> ArchitectureException {
    ArchitectureException {
        rule_id: rule_id.to_owned(),
        scope: scope.to_owned(),
        rationale: String::from("fixture exception"),
        owner: String::from("architecture-test"),
        expires: Some(String::from("2099-01-01")),
        tracking_task: None,
    }
}

pub(super) fn scan_dynamic_queries(
    source: &str,
    exceptions: &[ArchitectureException],
) -> Vec<Diagnostic> {
    let root = tempdir().expect("temporary scanner root");
    let source_path = root.path().join("src/lib.rs");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("create source parent");
    fs::write(&source_path, source).expect("write source");
    let active = BTreeSet::from([STATIC_RULE]);
    let exception_refs = exceptions.iter().collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    validate_rust_source(
        root.path(),
        Path::new("src/lib.rs"),
        &source_path,
        &active,
        &exception_refs,
        &mut diagnostics,
    );
    diagnostics
}

pub(super) fn scope_result(source: &str, scope: &str) -> Result<(), &'static str> {
    let root = tempdir().expect("temporary scope root");
    let source_path = root.path().join("src/lib.rs");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("create source parent");
    fs::write(&source_path, source).expect("write source");
    validate_exception_scope(root.path(), scope)
}

pub(super) fn scan_pagination_fixture(name: &str) -> Vec<Diagnostic> {
    let root = fixture(name);
    let source_path = root.join("src/lib.rs");
    let source = fs::read_to_string(&source_path).expect("pagination fixture source");
    let active = BTreeSet::from([PAGINATION_RULE]);
    let mut diagnostics = Vec::new();
    validate_rust_source(
        &root,
        Path::new("src/lib.rs"),
        &source_path,
        &active,
        &[],
        &mut diagnostics,
    );
    assert!(!source.is_empty());
    diagnostics
}

pub(super) fn scan_pagination_source(source: &str, declaration: Option<&str>) -> Vec<Diagnostic> {
    scan_pagination_source_with_migration(source, declaration, None)
}

pub(super) fn scan_pagination_source_with_migration(
    source: &str,
    declaration: Option<&str>,
    migration: Option<&str>,
) -> Vec<Diagnostic> {
    let root = tempdir().expect("temporary pagination root");
    let source_path = root.path().join("src/lib.rs");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("create source parent");
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\n",
    )
    .expect("write fixture manifest");
    fs::write(&source_path, source).expect("write pagination source");
    if let Some(declaration) = declaration {
        fs::write(root.path().join("pagination.toml"), declaration)
            .expect("write pagination declaration");
    }
    if let Some(migration) = migration {
        fs::create_dir_all(root.path().join("migrations")).expect("create migrations");
        fs::write(
            root.path()
                .join("migrations/0061_run_provenance_inspection.sql"),
            migration,
        )
        .expect("write migration");
    }
    let active = BTreeSet::from([PAGINATION_RULE]);
    let mut diagnostics = Vec::new();
    validate_rust_source(
        root.path(),
        Path::new("src/lib.rs"),
        &source_path,
        &active,
        &[],
        &mut diagnostics,
    );
    diagnostics
}

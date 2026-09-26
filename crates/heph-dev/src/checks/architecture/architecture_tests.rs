use super::{
    ArchitectureConfiguration, ArchitectureException, CargoDependency, CargoMetadata, CargoPackage,
    Diagnostic, REQUIRED_RULE_IDS,
    architecture_exceptions::{usable_exceptions, validate_exact_scope},
    architecture_io::read_configuration,
    architecture_metadata::validate_metadata,
    architecture_registry::{known_rule_ids, migration_gated_rule_count},
    architecture_validation::validate_repository,
};
use serde_json::Value;
use std::{fs, path::Path};
use tempfile::tempdir;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/architecture")
        .join(name)
}

fn valid_document(configuration: &ArchitectureConfiguration) -> String {
    known_rule_ids()
            .into_iter()
            .map(|rule_id| {
                let state = if configuration
                    .enabled_rules
                    .iter()
                    .any(|enabled| enabled == rule_id)
                {
                    "harness"
                } else {
                    "migration-gated"
                };
                format!(
                    "| `{rule_id}` | Structural / {state} | Fixture rationale and scope. | `cargo dev check architecture` | Fixture remediation. |"
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
}

#[test]
fn migration_gated_count_excludes_harness_only_rules() {
    let enabled_rules = vec![
        "ARCH-RULE-REGISTRY".to_owned(),
        "UI-TIER-DIRECTION".to_owned(),
        "UI-PAGE-STATE-COVERAGE".to_owned(),
    ];

    assert_eq!(
        migration_gated_rule_count(&enabled_rules),
        REQUIRED_RULE_IDS.len() - 2
    );
}

#[test]
fn valid_fixture_has_a_narrow_accountable_exception() {
    let root = fixture("valid");
    let configuration = read_configuration(&root).expect("valid fixture configuration");
    assert_eq!(configuration.exceptions.len(), 1);
    assert!(validate_exact_scope(&root, &configuration.exceptions[0].scope).is_ok());

    let document = valid_document(&configuration);
    let metadata = CargoMetadata {
        packages: vec![CargoPackage {
            id: "fixture 0.1.0".into(),
            name: "fixture".into(),
            manifest_path: root.join("src/lib.rs"),
            metadata: Value::Null,
            dependencies: Vec::new(),
        }],
        workspace_members: vec!["fixture 0.1.0".into()],
        workspace_root: root.clone(),
    };
    assert!(validate_repository(&root, &document, &configuration, &metadata).is_empty());
}

#[test]
fn invalid_fixture_rejects_directory_wide_exception() {
    let root = fixture("invalid-directory-exception");
    let configuration: ArchitectureConfiguration =
        read_configuration(&root).expect("syntactically valid fixture configuration");
    let error = validate_exact_scope(&root, &configuration.exceptions[0].scope)
        .expect_err("directory scope must fail");
    assert_eq!(error, "directory-wide exceptions are forbidden");
}

#[test]
fn invalid_fixture_produces_linked_rule_diagnostic() {
    let root = fixture("invalid-directory-exception");
    let configuration = read_configuration(&root).expect("fixture configuration parses");
    let metadata = CargoMetadata {
        packages: Vec::new(),
        workspace_members: Vec::new(),
        workspace_root: root.clone(),
    };
    let diagnostics = validate_repository(&root, "", &configuration, &metadata);
    let rendered = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule_id == "ARCH-EXCEPTION-FORMAT")
        .expect("exception diagnostic")
        .render();
    assert!(rendered.contains("[ARCH-EXCEPTION-FORMAT]"));
    assert!(rendered.contains("ARCHITECTURE.md#architecture-rule-index"));
}

#[test]
fn rule_index_rejects_unknown_and_duplicate_rows() {
    let root = fixture("valid");
    let configuration = read_configuration(&root).expect("fixture configuration parses");
    let document = format!(
        "{}\n| `ARCH-RULE-REGISTRY` | Structural / harness | Duplicate. | `cargo dev check architecture` | Remove it. |\n| `ARCH-NOT-REGISTERED` | Structural / migration-gated | Unknown. | `cargo dev check architecture` | Register it. |",
        valid_document(&configuration)
    );
    let metadata = CargoMetadata {
        packages: Vec::new(),
        workspace_members: Vec::new(),
        workspace_root: root.clone(),
    };
    let diagnostics = validate_repository(&root, &document, &configuration, &metadata);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("ARCH-RULE-REGISTRY more than once")
    }));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("ARCH-NOT-REGISTERED"))
    );
}

#[test]
fn invalid_fixture_rejects_workspace_wide_exception() {
    let root = fixture("invalid-workspace-exception");
    let configuration: ArchitectureConfiguration =
        read_configuration(&root).expect("syntactically valid fixture configuration");
    assert!(validate_exact_scope(&root, &configuration.exceptions[0].scope).is_err());
}

#[test]
fn cargo_metadata_fixture_rejects_a_missing_member_record() {
    let root = fixture("valid");
    let metadata: CargoMetadata = serde_json::from_str(&format!(
        r#"{{"packages":[],"workspace_members":["missing"],"workspace_root":{}}}"#,
        serde_json::to_string(&root).expect("path serializes")
    ))
    .expect("metadata fixture parses");
    let mut diagnostics = Vec::<Diagnostic>::new();
    validate_metadata(&root, &metadata, &mut diagnostics);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "ARCH-CARGO-METADATA"
            && diagnostic.message.contains("no package record")
    }));
}

#[test]
fn invalid_exception_fixture_covers_accountability_and_exact_scope_failures() {
    let root = fixture("invalid-exceptions");
    let configuration = read_configuration(&root).expect("fixture configuration parses");
    let metadata = CargoMetadata {
        packages: Vec::new(),
        workspace_members: Vec::new(),
        workspace_root: root.clone(),
    };
    let diagnostics = validate_repository(&root, "", &configuration, &metadata);
    let messages = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule_id == "ARCH-EXCEPTION-FORMAT")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    for expected in [
        "unknown rule",
        "expiry or tracking task",
        "globs are forbidden",
        "beyond the end",
        "item does not exist",
        "has expired",
        "duplicates exact scope",
        "unresolved tracking task",
    ] {
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "missing diagnostic containing {expected}"
        );
    }
}

#[test]
fn missing_required_exception_field_is_rejected_during_parsing() {
    let root = fixture("invalid-missing-field");
    let error = read_configuration(&root).expect_err("missing owner must fail");
    assert!(error.to_string().contains("missing field `owner`"));
}

#[test]
fn cargo_metadata_fixture_rejects_repository_escaping_path_dependency() {
    let root = fixture("valid");
    let package_id = "fixture 0.1.0".to_owned();
    let metadata = CargoMetadata {
        packages: vec![CargoPackage {
            id: package_id.clone(),
            name: "fixture".into(),
            manifest_path: root.join("src/lib.rs"),
            metadata: Value::Null,
            dependencies: vec![CargoDependency {
                name: "escape".into(),
                path: Some(Path::new("/tmp").to_path_buf()),
                kind: None,
            }],
        }],
        workspace_members: vec![package_id],
        workspace_root: root.clone(),
    };
    let mut diagnostics = Vec::new();
    validate_metadata(&root, &metadata, &mut diagnostics);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "ARCH-CARGO-METADATA"
            && diagnostic.message.contains("repository-escaping")
    }));
}

#[test]
fn impossible_calendar_expiry_is_rejected() {
    assert!(!super::architecture_exceptions::date_shape("2026-02-29"));
    assert!(!super::architecture_exceptions::date_shape("2026-13-01"));
    assert!(super::architecture_exceptions::date_shape("2028-02-29"));
}

#[test]
fn invalid_exception_records_are_not_usable_by_scanners() {
    let root = fixture("valid");
    let configuration = ArchitectureConfiguration {
        version: 1,
        enabled_rules: Vec::new(),
        maximum_file_lines: std::iter::once((String::from("domain"), 500)).collect(),
        exceptions: vec![ArchitectureException {
            rule_id: String::from("DB-STATIC-SQL"),
            scope: String::from("src/lib.rs#GENERATED_TABLE"),
            rationale: String::from("expired fixture exception"),
            owner: String::from("architecture-test"),
            expires: Some(String::from("2020-01-01")),
            tracking_task: None,
        }],
    };
    assert!(usable_exceptions(&root, &configuration).is_empty());
}

#[test]
fn usable_exceptions_accepts_a_valid_module_qualified_db_item() {
    let root = tempdir().expect("temporary repository root");
    let source_path = root.path().join("src/lib.rs");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("create source parent");
    fs::write(
        &source_path,
        "mod one { struct Thing; impl Thing { fn run() {} } }\n",
    )
    .expect("write Rust source");
    let configuration = ArchitectureConfiguration {
        version: 1,
        enabled_rules: Vec::new(),
        maximum_file_lines: std::iter::once((String::from("domain"), 500)).collect(),
        exceptions: vec![ArchitectureException {
            rule_id: String::from("DB-STATIC-SQL"),
            scope: String::from("src/lib.rs#one::Thing::run"),
            rationale: String::from("qualified fixture exception"),
            owner: String::from("architecture-test"),
            expires: Some(String::from("2099-01-01")),
            tracking_task: None,
        }],
    };
    assert_eq!(usable_exceptions(root.path(), &configuration).len(), 1);
}

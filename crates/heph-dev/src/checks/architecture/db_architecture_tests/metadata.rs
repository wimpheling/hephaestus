use super::super::{RULES, audit, contains_schema_sql};
use super::support::{
    dev_sqlx_dependency, fixture, package, path_dependency, sqlx_dependency,
    sqlx_metadata_diagnostics,
};
use crate::checks::architecture::CargoMetadata;
use serde_json::json;

#[test]
fn explicitly_marked_dev_sqlx_harness_is_allowed() {
    let root = fixture("valid");
    let harness = package(
        &root,
        "harness",
        json!({"hephaestus": {"sqlx_test_dependency": true}}),
        vec![dev_sqlx_dependency()],
    );
    let metadata = CargoMetadata {
        workspace_members: vec![harness.id.clone()],
        packages: vec![harness],
        workspace_root: root.clone(),
    };
    assert!(audit(&root, &metadata, &[]).is_empty());
}

#[test]
fn valid_adapter_is_a_capability_firewall_and_owns_only_static_queries() {
    let root = fixture("valid");
    let adapter = package(
        &root,
        "adapter-postgres",
        json!({"hephaestus": {"postgres_adapter": true, "database_context": "fixture"}}),
        vec![sqlx_dependency()],
    );
    let application = package(
        &root,
        "application",
        serde_json::Value::Null,
        vec![path_dependency(
            "adapter-postgres",
            root.join("adapter-postgres"),
        )],
    );
    let metadata = CargoMetadata {
        workspace_members: vec![adapter.id.clone(), application.id.clone()],
        packages: vec![adapter, application],
        workspace_root: root.clone(),
    };
    assert!(audit(&root, &metadata, &[]).is_empty());
}
#[test]
fn production_postgres_adapter_requires_the_postgres_package_suffix() {
    let root = fixture("invalid");
    let adapter = package(
        &root,
        "adapter",
        json!({"hephaestus": {"postgres_adapter": true, "database_context": "fixture"}}),
        vec![sqlx_dependency()],
    );
    let metadata = CargoMetadata {
        workspace_members: vec![adapter.id.clone()],
        packages: vec![adapter],
        workspace_root: root,
    };

    let diagnostics = sqlx_metadata_diagnostics(&metadata);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("must use a package name ending in `-postgres`")
    }));
}

#[test]
fn postgres_suffix_without_adapter_metadata_does_not_authorize_sqlx() {
    let root = fixture("invalid");
    let suffix_package = package(
        &root,
        "suffix-without-metadata-postgres",
        serde_json::Value::Null,
        vec![sqlx_dependency()],
    );
    let consumer = package(
        &root,
        "suffix-consumer",
        serde_json::Value::Null,
        vec![path_dependency(
            "suffix-without-metadata-postgres",
            root.join("suffix-without-metadata-postgres"),
        )],
    );
    let metadata = CargoMetadata {
        workspace_members: vec![suffix_package.id.clone(), consumer.id.clone()],
        packages: vec![suffix_package, consumer],
        workspace_root: root,
    };

    let diagnostics = sqlx_metadata_diagnostics(&metadata);
    assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains(
                "workspace package suffix-without-metadata-postgres reaches SQLx outside a declared PostgreSQL adapter",
            )
        }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains(
            "workspace package suffix-consumer reaches SQLx outside a declared PostgreSQL adapter",
        )
    }));
}

#[test]
fn invalid_fixture_covers_direct_transitive_metadata_ddl_and_dynamic_sql() {
    let root = fixture("invalid");
    let direct = package(
        &root,
        "direct",
        serde_json::Value::Null,
        vec![sqlx_dependency()],
    );
    let consumer = package(
        &root,
        "consumer",
        serde_json::Value::Null,
        vec![path_dependency("direct", root.join("direct"))],
    );
    let invalid_adapter = package(
        &root,
        "invalid-adapter",
        json!({"hephaestus": {"postgres_adapter": true, "database_context": "fixture"}}),
        vec![sqlx_dependency()],
    );
    let metadata = CargoMetadata {
        workspace_members: vec![
            direct.id.clone(),
            consumer.id.clone(),
            invalid_adapter.id.clone(),
        ],
        packages: vec![direct, consumer, invalid_adapter],
        workspace_root: root.clone(),
    };
    let counts = audit(&root, &metadata, &[]);
    assert_eq!(counts.get(RULES[0]), Some(&4));
    assert_eq!(counts.get(RULES[1]), Some(&4));
    assert_eq!(counts.get(RULES[2]), Some(&3));
}

#[test]
fn schema_classifier_is_case_and_layout_insensitive() {
    assert!(contains_schema_sql("create\n table example(id int)"));
    assert!(contains_schema_sql("ALTER TYPE status ADD VALUE 'done'"));
    assert!(!contains_schema_sql("SELECT * FROM example"));
}

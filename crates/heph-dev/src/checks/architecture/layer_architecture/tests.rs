use super::validate;
use crate::checks::architecture::{CargoDependency, CargoMetadata, CargoPackage, Diagnostic};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct FixtureGraph {
    packages: Vec<FixturePackage>,
}

#[derive(Debug, Deserialize)]
struct FixturePackage {
    id: String,
    layer: String,
    context: String,
    dependencies: Vec<String>,
}

fn package(id: &str, layer: &str, context: &str, dependencies: &[(&str, &str)]) -> CargoPackage {
    CargoPackage {
        id: id.to_owned(),
        name: id.to_owned(),
        manifest_path: PathBuf::from(format!("/{id}/Cargo.toml")),
        metadata: json!({"hephaestus": {"layer": layer, "context": context}}),
        dependencies: dependencies
            .iter()
            .map(|(name, path)| CargoDependency {
                name: (*name).to_owned(),
                path: Some(PathBuf::from(format!("/{path}"))),
                kind: None,
            })
            .collect(),
    }
}

fn graph(packages: Vec<CargoPackage>) -> CargoMetadata {
    CargoMetadata {
        workspace_members: packages.iter().map(|package| package.id.clone()).collect(),
        packages,
        workspace_root: PathBuf::from("/"),
    }
}

fn read_fixture<T: DeserializeOwned>(name: &str) -> T {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/architecture-graph")
        .join(name)
        .join("graph.toml");
    toml::from_str(&std::fs::read_to_string(path).expect("architecture graph fixture"))
        .expect("architecture graph fixture parses")
}

fn fixture_metadata(name: &str) -> CargoMetadata {
    let fixture: FixtureGraph = read_fixture(name);
    let workspace_members = fixture
        .packages
        .iter()
        .map(|package| package.id.clone())
        .collect::<Vec<_>>();
    let packages = fixture
        .packages
        .into_iter()
        .map(|package| {
            let dependencies = package
                .dependencies
                .into_iter()
                .map(|dependency| CargoDependency {
                    name: dependency.clone(),
                    path: Some(PathBuf::from(format!("/{dependency}"))),
                    kind: None,
                })
                .collect();
            CargoPackage {
                id: package.id.clone(),
                name: package.id.clone(),
                manifest_path: PathBuf::from(format!("/{}/Cargo.toml", package.id)),
                metadata: json!({
                    "hephaestus": {
                        "layer": package.layer,
                        "context": package.context,
                    }
                }),
                dependencies,
            }
        })
        .collect();
    CargoMetadata {
        packages,
        workspace_members,
        workspace_root: PathBuf::from("/"),
    }
}

#[test]
fn valid_graph_has_no_diagnostics() {
    let metadata = graph(vec![
        package("domain", "domain", "orders", &[]),
        package("adapter", "adapter", "orders", &[("domain", "domain")]),
    ]);
    let mut diagnostics = Vec::<Diagnostic>::new();
    validate(
        &["ARCH-CRATE-LAYERS".to_owned()],
        &metadata,
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn upward_dependency_is_actionable() {
    let metadata = graph(vec![
        package("domain", "domain", "orders", &[("adapter", "adapter")]),
        package("adapter", "adapter", "orders", &[]),
    ]);
    let mut diagnostics = Vec::new();
    validate(
        &["ARCH-CRATE-LAYERS".to_owned()],
        &metadata,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("upward layer dependency")
            && diagnostic.message.contains("domain")
    }));
}

#[test]
fn cross_context_adapter_requires_an_exact_declaration() {
    let metadata = graph(vec![
        package("left", "adapter", "left", &[("right", "right")]),
        package("right", "adapter", "right", &[]),
    ]);
    let mut diagnostics = Vec::new();
    validate(
        &["ARCH-CRATE-LAYERS".to_owned()],
        &metadata,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cross-context adapter dependency")
    }));
}

#[test]
fn cycle_is_reported_with_the_package_path() {
    let metadata = graph(vec![
        package("left", "adapter", "left", &[("right", "right")]),
        package("right", "adapter", "right", &[("left", "left")]),
    ]);
    let mut diagnostics = Vec::new();
    validate(
        &["ARCH-CRATE-LAYERS".to_owned()],
        &metadata,
        &mut diagnostics,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("dependency cycle detected"))
    );
}

#[test]
fn graph_fixtures_cover_valid_cycle_upward_and_context_edges() {
    let mut diagnostics = Vec::new();
    validate(
        &["ARCH-CRATE-LAYERS".to_owned()],
        &fixture_metadata("valid"),
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty(), "valid fixture: {diagnostics:?}");

    for (fixture, expected) in [
        ("invalid-cycle", "dependency cycle detected"),
        ("invalid-upward", "upward layer dependency"),
        ("invalid-cross-context", "cross-context adapter dependency"),
    ] {
        let mut diagnostics = Vec::new();
        validate(
            &["ARCH-CRATE-LAYERS".to_owned()],
            &fixture_metadata(fixture),
            &mut diagnostics,
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "{fixture} missing {expected}: {diagnostics:?}"
        );
    }
}

#[test]
fn domain_runtime_and_provider_boundaries_are_actionable() {
    let metadata = graph(vec![
        package("domain", "domain", "orders", &[("tokio", "tokio")]),
        package(
            "application",
            "application",
            "orders",
            &[("vm-fake", "vm-fake")],
        ),
    ]);
    let mut diagnostics = Vec::new();
    validate(
        &["ARCH-CRATE-LAYERS".to_owned()],
        &metadata,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("domain package domain") && diagnostic.message.contains("tokio")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("reusable package application")
            && diagnostic.message.contains("vm-fake")
    }));
}

#[test]
fn generated_rpc_types_are_only_boundary_dependencies() {
    let metadata = graph(vec![package(
        "application",
        "application",
        "orders",
        &[("rpc-proto", "rpc-proto")],
    )]);
    let mut diagnostics = Vec::new();
    validate(
        &["ARCH-CRATE-LAYERS".to_owned()],
        &metadata,
        &mut diagnostics,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("imports generated RPC transport types")
    }));
}

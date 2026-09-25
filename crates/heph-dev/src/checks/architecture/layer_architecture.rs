//! Cargo metadata checks for the Rust layer and bounded-context graph.

use super::{CargoMetadata, CargoPackage, Diagnostic};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const RULE: &str = "ARCH-CRATE-LAYERS";
const LAYERS: [&str; 8] = [
    "domain",
    "port",
    "application",
    "adapter",
    "worker",
    "transport",
    "composition",
    "development",
];
const DOMAIN_FORBIDDEN_DEPENDENCIES: [&str; 14] = [
    "async-nats",
    "axum",
    "connectrpc",
    "connectrpc-reflection",
    "futures-util",
    "hyper",
    "libc",
    "libloading",
    "reqwest",
    "rusqlite",
    "rustix",
    "sqlx",
    "tokio",
    "tokio-util",
];
const VM_PROVIDER_CRATES: [&str; 2] = ["vm-fake", "vm-libkrun"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Declaration {
    layer: String,
    context: String,
    allow_cross_context_dependencies: BTreeSet<String>,
}

// Keep graph validation together so diagnostics follow one dependency-edge pass.
#[allow(clippy::too_many_lines)]
pub(super) fn validate(
    enabled_rules: &[String],
    metadata: &CargoMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !enabled_rules.iter().any(|rule| rule == RULE) {
        return;
    }

    let workspace_ids = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
    let packages = metadata
        .packages
        .iter()
        .filter(|package| workspace_ids.contains(&package.id))
        .collect::<Vec<_>>();
    let mut declarations = BTreeMap::new();
    for package in &packages {
        let Some(declaration) = declaration(package, diagnostics) else {
            continue;
        };
        declarations.insert(package.id.clone(), declaration);
    }

    let packages_by_path = packages
        .iter()
        .map(|package| (manifest_root(package), package.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut graph = BTreeMap::<String, Vec<String>>::new();
    for package in &packages {
        let Some(source) = declarations.get(&package.id) else {
            continue;
        };
        let edges = graph.entry(package.id.clone()).or_default();
        let mut dependency_names = BTreeSet::new();
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
        {
            dependency_names.insert(dependency.name.as_str());
            if source.layer == "domain"
                && DOMAIN_FORBIDDEN_DEPENDENCIES.contains(&dependency.name.as_str())
            {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "domain package {} depends on forbidden runtime/transport/persistence crate {}; keep the domain independent or document a narrow invariant",
                        package.name, dependency.name
                    ),
                ));
            }
            if source.layer != "composition"
                && source.layer != "development"
                && VM_PROVIDER_CRATES.contains(&dependency.name.as_str())
            {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "reusable package {} imports VM provider {}; depend on vm-trait instead",
                        package.name, dependency.name
                    ),
                ));
            }
            if dependency.name == "rpc-proto"
                && source.layer != "transport"
                && source.layer != "composition"
            {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "package {} imports generated RPC transport types from {}; convert at a transport or composition boundary",
                        package.name, dependency.name
                    ),
                ));
            }
            let Some(path) = dependency.path.as_ref() else {
                continue;
            };
            let Some(target_id) = packages_by_path.get(&canonical(path)) else {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "workspace package {} has an undeclared path dependency {}; add it to workspace metadata",
                        package.name, dependency.name
                    ),
                ));
                continue;
            };
            let Some(target) = declarations.get(target_id) else {
                continue;
            };
            edges.push(target_id.clone());
            if layer_rank(&source.layer) < layer_rank(&target.layer) {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "upward layer dependency {} ({}) -> {} ({}); depend on an inner port or move the edge outward",
                        package.name, source.layer, dependency.name, target.layer
                    ),
                ));
            }
            if source.context != target.context
                && target.layer == "adapter"
                && source.layer != "composition"
                && !source
                    .allow_cross_context_dependencies
                    .contains(&dependency.name)
            {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "cross-context adapter dependency {} ({}) -> {} ({}); declare `allow_cross_context_dependencies` for this exact edge or depend on an application port",
                        package.name, source.context, dependency.name, target.context
                    ),
                ));
            }
        }
        for allowed in &source.allow_cross_context_dependencies {
            if !dependency_names.contains(allowed.as_str()) {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "workspace package {} allows cross-context dependency {allowed}, but no production path dependency has that name",
                        package.name
                    ),
                ));
            }
        }
        if VM_PROVIDER_CRATES.contains(&package.name.as_str())
            && !dependency_names.contains("vm-trait")
        {
            diagnostics.push(Diagnostic::new(
                RULE,
                format!(
                    "VM provider {} must implement the vm-trait contract",
                    package.name
                ),
            ));
        }
    }
    detect_cycles(&graph, &packages, diagnostics);
}

fn declaration(package: &CargoPackage, diagnostics: &mut Vec<Diagnostic>) -> Option<Declaration> {
    let Some(hephaestus) = package.metadata.get("hephaestus") else {
        diagnostics.push(Diagnostic::new(
            RULE,
            format!(
                "workspace package {} lacks `package.metadata.hephaestus.layer` and `.context`",
                package.name
            ),
        ));
        return None;
    };
    let Some(layer) = hephaestus.get("layer").and_then(Value::as_str) else {
        diagnostics.push(Diagnostic::new(
            RULE,
            format!(
                "workspace package {} has no string architecture layer",
                package.name
            ),
        ));
        return None;
    };
    if !LAYERS.contains(&layer) {
        diagnostics.push(Diagnostic::new(
            RULE,
            format!(
                "workspace package {} declares unknown architecture layer `{layer}`",
                package.name
            ),
        ));
    }
    let Some(context) = hephaestus.get("context").and_then(Value::as_str) else {
        diagnostics.push(Diagnostic::new(
            RULE,
            format!(
                "workspace package {} has no string bounded context",
                package.name
            ),
        ));
        return None;
    };
    if context.is_empty()
        || !context
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        diagnostics.push(Diagnostic::new(
            RULE,
            format!(
                "workspace package {} declares invalid bounded context `{context}`",
                package.name
            ),
        ));
    }
    let allow_cross_context_dependencies = hephaestus
        .get("allow_cross_context_dependencies")
        .and_then(Value::as_array)
        .map_or_else(BTreeSet::new, |values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        });
    Some(Declaration {
        layer: layer.to_owned(),
        context: context.to_owned(),
        allow_cross_context_dependencies,
    })
}

fn detect_cycles(
    graph: &BTreeMap<String, Vec<String>>,
    packages: &[&CargoPackage],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let names = packages
        .iter()
        .map(|package| (package.id.clone(), package.name.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut visited = BTreeSet::new();
    let mut active = BTreeSet::new();
    let mut stack = Vec::new();
    for node in graph.keys() {
        detect_cycle_from(
            node,
            graph,
            &names,
            &mut visited,
            &mut active,
            &mut stack,
            diagnostics,
        );
    }
}

fn detect_cycle_from(
    node: &str,
    graph: &BTreeMap<String, Vec<String>>,
    names: &BTreeMap<String, String>,
    visited: &mut BTreeSet<String>,
    active: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if active.contains(node) {
        let start = stack.iter().position(|entry| entry == node).unwrap_or(0);
        let cycle = stack[start..]
            .iter()
            .chain(std::iter::once(&node.to_owned()))
            .filter_map(|id| names.get(id))
            .cloned()
            .collect::<Vec<_>>();
        diagnostics.push(Diagnostic::new(
            RULE,
            format!("dependency cycle detected: {}", cycle.join(" -> ")),
        ));
        return;
    }
    if !visited.insert(node.to_owned()) {
        return;
    }
    active.insert(node.to_owned());
    stack.push(node.to_owned());
    if let Some(edges) = graph.get(node) {
        for edge in edges {
            detect_cycle_from(edge, graph, names, visited, active, stack, diagnostics);
        }
    }
    stack.pop();
    active.remove(node);
}

fn layer_rank(layer: &str) -> u8 {
    match layer {
        "domain" | "port" => 0,
        "application" => 1,
        "adapter" | "worker" => 2,
        "transport" => 3,
        "composition" => 4,
        "development" => 5,
        _ => u8::MAX,
    }
}

fn manifest_root(package: &CargoPackage) -> PathBuf {
    canonical(
        package
            .manifest_path
            .parent()
            .unwrap_or(&package.manifest_path),
    )
}

fn canonical(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
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

    fn package(
        id: &str,
        layer: &str,
        context: &str,
        dependencies: &[(&str, &str)],
    ) -> CargoPackage {
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
            diagnostic.message.contains("domain package domain")
                && diagnostic.message.contains("tokio")
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
}

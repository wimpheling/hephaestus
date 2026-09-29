//! Cargo metadata checks for the Rust layer and bounded-context graph.

mod graph;

use super::{CargoMetadata, CargoPackage, Diagnostic};
use graph::{canonical, declaration, detect_cycles, layer_rank, manifest_root};
use std::collections::{BTreeMap, BTreeSet};

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

#[cfg(test)]
#[path = "layer_architecture/tests.rs"]
mod tests;

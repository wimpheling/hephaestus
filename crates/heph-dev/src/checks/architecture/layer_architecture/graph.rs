use super::{CargoPackage, Diagnostic, LAYERS, RULE};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Declaration {
    pub(super) layer: String,
    pub(super) context: String,
    pub(super) allow_cross_context_dependencies: BTreeSet<String>,
}

pub(super) fn declaration(
    package: &CargoPackage,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Declaration> {
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

pub(super) fn detect_cycles(
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

pub(super) fn layer_rank(layer: &str) -> u8 {
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

pub(super) fn manifest_root(package: &CargoPackage) -> PathBuf {
    canonical(
        package
            .manifest_path
            .parent()
            .unwrap_or(&package.manifest_path),
    )
}

pub(super) fn canonical(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

//! Semantic Rust boundary checks that cannot be derived from Cargo metadata.

use super::Diagnostic;
use std::{collections::BTreeMap, path::Path};

mod rules;
mod sensitive_flow;
mod source_scan;

const RULES: [&str; 8] = [
    "ARCH-ENV-ONLY-IN-CONFIG",
    "ARCH-HTTP-ONLY-IN-INTEGRATIONS",
    "ARCH-PROCESS-ONLY-IN-ADAPTERS",
    "ARCH-FILESYSTEM-ONLY-IN-ADAPTERS",
    "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT",
    "SEC-NO-SENSITIVE-LOG-ARGUMENTS",
    "SEC-SENTINEL-NO-PLAINTEXT",
    "ARCH-VM-PROVIDER-ONLY-IN-COMPOSITION",
];

pub(super) fn validate(root: &Path, enabled_rules: &[String], diagnostics: &mut Vec<Diagnostic>) {
    let active = RULES
        .into_iter()
        .filter(|rule| enabled_rules.iter().any(|enabled| enabled == rule))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return;
    }
    source_scan::visit_sources(root, &root.join("crates"), &active, diagnostics);
    if active.contains(&"SEC-SENTINEL-NO-PLAINTEXT") {
        source_scan::scan_repository_sentinels(root, diagnostics);
    }
}

pub(super) fn audit(root: &Path) -> BTreeMap<&'static str, usize> {
    let mut diagnostics = Vec::new();
    source_scan::visit_sources(root, &root.join("crates"), &RULES, &mut diagnostics);
    source_scan::scan_repository_sentinels(root, &mut diagnostics);
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
#[path = "rust_architecture/tests.rs"]
mod tests;

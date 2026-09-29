//! Migration-gated structural checks for durable product events.

use super::Diagnostic;
use std::{collections::BTreeMap, path::Path};

mod consumer;
mod contract;
mod source_scan;
mod transaction;

const RULES: [&str; 9] = [
    "EVT-CANONICAL-ENVELOPE",
    "EVT-CONSUMER-USES-INBOX",
    "EVT-NATS-ONLY-IN-EVENT-ADAPTERS",
    "EVT-OUTBOX-PUBLISHER-ONLY",
    "EVT-REDUCER-COVERAGE",
    "EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM",
    "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY",
    "EVT-STREAM-REAUTHORIZATION",
    "EVT-TYPED-ONEOF-PAYLOAD",
];

const PAYLOAD_VARIANTS: [&str; 18] = [
    "identity_organizations_changed",
    "organization_changed",
    "project_changed",
    "repository_changed",
    "repository_ref_changed",
    "build_changed",
    "release_changed",
    "agent_instance_changed",
    "run_changed",
    "review_changed",
    "secret_metadata_changed",
    "secret_grant_changed",
    "secret_import_changed",
    "agent_secret_binding_changed",
    "artifact_changed",
    "identity_profile_changed",
    "registry_publication_changed",
    "gateway_changed",
];

pub(super) fn validate(root: &Path, enabled_rules: &[String], diagnostics: &mut Vec<Diagnostic>) {
    let active = RULES
        .into_iter()
        .filter(|rule| enabled_rules.iter().any(|enabled| enabled == rule))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return;
    }
    contract::validate_contract_files(root, &active, diagnostics);
    source_scan::visit_rust_sources(root, &root.join("crates"), &active, diagnostics);
}

pub(super) fn audit(root: &Path) -> BTreeMap<&'static str, usize> {
    let active = RULES.to_vec();
    let mut diagnostics = Vec::new();
    contract::validate_contract_files(root, &active, &mut diagnostics);
    source_scan::visit_rust_sources(root, &root.join("crates"), &active, &mut diagnostics);
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
#[path = "event_architecture/tests.rs"]
mod tests;

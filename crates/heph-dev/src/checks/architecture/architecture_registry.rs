use std::collections::BTreeSet;

use super::{ArchitectureConfiguration, DOCUMENT, Diagnostic, HARNESS_RULE_IDS, REQUIRED_RULE_IDS};

pub(super) fn migration_gated_rule_count(enabled_rules: &[String]) -> usize {
    let enabled_catalogue_rules = enabled_rules
        .iter()
        .filter(|rule_id| REQUIRED_RULE_IDS.contains(&rule_id.as_str()))
        .count();
    REQUIRED_RULE_IDS.len() - enabled_catalogue_rules
}
pub(super) fn validate_rule_registry(
    document: &str,
    configuration: &ArchitectureConfiguration,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let known_rules = known_rule_ids();
    let mut indexed_rules = BTreeSet::new();
    for line in document.lines().filter(|line| line.starts_with("| `")) {
        let cells = line
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        let Some(rule_id) = cells
            .first()
            .and_then(|cell| cell.strip_prefix('`'))
            .and_then(|cell| cell.strip_suffix('`'))
        else {
            continue;
        };
        if !known_rules.contains(rule_id) {
            diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("{DOCUMENT} indexes unknown invariant {rule_id}"),
            ));
            continue;
        }
        if !indexed_rules.insert(rule_id) {
            diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("{DOCUMENT} indexes {rule_id} more than once"),
            ));
        }
        validate_index_row(rule_id, &cells, configuration, diagnostics);
    }
    for rule_id in known_rules.difference(&indexed_rules) {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} does not index required invariant {rule_id}"),
        ));
    }

    let mut seen = BTreeSet::new();
    for rule_id in &configuration.enabled_rules {
        if !known_rule_ids().contains(rule_id.as_str()) {
            diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("enabled rule {rule_id} is absent from the stable registry"),
            ));
        }
        if !seen.insert(rule_id) {
            diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("enabled rule {rule_id} is listed more than once"),
            ));
        }
    }
}

pub(super) fn validate_index_row(
    rule_id: &str,
    cells: &[&str],
    configuration: &ArchitectureConfiguration,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if cells.len() != 5 {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} must have exactly five index columns"),
        ));
        return;
    }
    let Some((class, state)) = cells[1].rsplit_once(" / ") else {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} lacks `Class / state`"),
        ));
        return;
    };
    if class.trim().is_empty() || !matches!(state, "harness" | "migration-gated") {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} has an invalid class or state"),
        ));
    }
    let expected_state = if configuration
        .enabled_rules
        .iter()
        .any(|enabled| enabled == rule_id)
    {
        "harness"
    } else {
        "migration-gated"
    };
    if state != expected_state {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!(
                "{DOCUMENT} row for {rule_id} has state {state}; configuration requires {expected_state}"
            ),
        ));
    }
    if cells[2].trim().is_empty() || cells[4].trim().is_empty() {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} requires rationale, scope, and remediation"),
        ));
    }
    let command = cells[3].trim_matches('`');
    if !matches!(
        command,
        "cargo dev check architecture"
            | "cargo dev check protobuf"
            | "cargo dev check rust"
            | "cargo dev check phoenix"
            | "cargo dev check ui"
            | "cargo dev check full"
    ) {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} has an unknown enforcement command"),
        ));
    }
}

pub(super) fn validate_configuration(
    configuration: &ArchitectureConfiguration,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if configuration.version != 1 {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!(
                "unsupported architecture configuration version {}; expected 1",
                configuration.version
            ),
        ));
    }
    if configuration.maximum_file_lines.is_empty()
        || configuration
            .maximum_file_lines
            .values()
            .any(|threshold| *threshold == 0)
    {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            "maximum file lengths must be configured centrally as positive line counts",
        ));
    }
}
pub(super) fn known_rule_ids() -> BTreeSet<&'static str> {
    HARNESS_RULE_IDS
        .into_iter()
        .chain(REQUIRED_RULE_IDS)
        .collect()
}

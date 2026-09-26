//! Static application-role transaction-context checks.

mod inventory;
mod resolver;
mod scanner;
mod source_scan;

use super::{CargoMetadata, Diagnostic};
use std::{collections::BTreeMap, path::Path};

pub(super) const RULE: &str = "DB-RLS-CONTEXT-REQUIRED";

pub(super) fn validate(
    root: &Path,
    enabled_rules: &[String],
    metadata: &CargoMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    source_scan::validate(root, enabled_rules, metadata, diagnostics);
}

pub(super) fn audit(root: &Path) -> BTreeMap<&'static str, usize> {
    source_scan::audit(root)
}

#[cfg(test)]
#[path = "db_rls/tests.rs"]
mod tests;

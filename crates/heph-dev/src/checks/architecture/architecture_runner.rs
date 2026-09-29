use std::{collections::BTreeMap, fs, path::Path};

use super::{
    DOCUMENT, Diagnostic, architecture_exceptions, architecture_io, architecture_registry,
    architecture_validation, db_architecture, db_rls, event_architecture, file_length_architecture,
    rpc_architecture, rust_architecture,
};
use crate::{
    context::DevContext,
    process::{DevError, Result},
};

pub fn run(context: &DevContext) -> Result<()> {
    let root = &context.repository_root;
    let document = fs::read_to_string(root.join(DOCUMENT)).map_err(|error| {
        DevError::Invalid(
            Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("cannot read {}: {error}", root.join(DOCUMENT).display()),
            )
            .render(),
        )
    })?;
    let configuration = architecture_io::read_configuration(root)?;
    let metadata = architecture_io::cargo_metadata(root)?;

    let diagnostics =
        architecture_validation::validate_repository(root, &document, &configuration, &metadata);
    if diagnostics.is_empty() {
        println!(
            "architecture checks passed ({} enabled; {} migration-gated)",
            configuration.enabled_rules.len(),
            architecture_registry::migration_gated_rule_count(&configuration.enabled_rules)
        );
        println!("enabled rules: {}", configuration.enabled_rules.join(", "));
        println!(
            "migration-gated families: ARCH, DB, EVT, RPC, SEC, UI, WEB; enable each rule only after its repository-wide migration"
        );
        let rpc_audit = rpc_architecture::audit(root);
        if rpc_audit.is_empty() {
            println!("migration-gated RPC structural dry-run: clean");
        } else {
            println!(
                "migration-gated RPC structural dry-run: {}",
                rpc_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let event_audit = event_architecture::audit(root);
        if event_audit.is_empty() {
            println!("migration-gated event structural dry-run: clean");
        } else {
            println!(
                "migration-gated event structural dry-run: {}",
                event_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let usable_exceptions = architecture_exceptions::usable_exceptions(root, &configuration);
        let database_audit = db_architecture::audit(root, &metadata, &usable_exceptions);
        if database_audit.is_empty() {
            println!("migration-gated database structural dry-run: clean");
        } else {
            println!(
                "migration-gated database structural dry-run: {}",
                database_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let rls_audit = db_rls::audit(root);
        if rls_audit.is_empty() {
            println!("migration-gated database RLS structural dry-run: clean");
        } else {
            println!(
                "migration-gated database RLS structural dry-run: {}",
                rls_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let rust_audit = rust_architecture::audit(root);
        if rust_audit.is_empty() {
            println!("migration-gated Rust semantic dry-run: clean");
        } else {
            println!(
                "migration-gated Rust semantic dry-run: {}",
                rust_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        print_file_length_audit(root, &configuration.maximum_file_lines);
        Ok(())
    } else {
        Err(DevError::Invalid(super::render_diagnostics(&diagnostics)))
    }
}

fn print_file_length_audit(root: &Path, maximum_file_lines: &BTreeMap<String, usize>) {
    let audit = file_length_architecture::audit(root, maximum_file_lines);
    if audit.is_empty() {
        println!("migration-gated Rust file-length dry-run: clean");
    } else {
        println!(
            "migration-gated Rust file-length dry-run: {}",
            audit
                .iter()
                .map(|(rule, count)| format!("{rule}={count}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

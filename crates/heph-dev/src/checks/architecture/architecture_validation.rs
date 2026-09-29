use std::path::Path;

use super::{
    ArchitectureConfiguration, CargoMetadata, Diagnostic,
    architecture_exceptions::{usable_exceptions, validate_exceptions},
    architecture_metadata::validate_metadata,
    architecture_registry::{validate_configuration, validate_rule_registry},
    db_architecture, db_rls, event_architecture, file_length_architecture, layer_architecture,
    rpc_architecture, rust_architecture, topology_architecture,
};

pub(super) fn validate_repository(
    root: &Path,
    document: &str,
    configuration: &ArchitectureConfiguration,
    metadata: &CargoMetadata,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    validate_rule_registry(document, configuration, &mut diagnostics);
    validate_configuration(configuration, &mut diagnostics);
    validate_exceptions(root, configuration, &mut diagnostics);
    let usable_exceptions = usable_exceptions(root, configuration);
    validate_metadata(root, metadata, &mut diagnostics);
    file_length_architecture::validate(
        root,
        &configuration.enabled_rules,
        &configuration.maximum_file_lines,
        &mut diagnostics,
    );
    layer_architecture::validate(&configuration.enabled_rules, metadata, &mut diagnostics);
    topology_architecture::validate(
        root,
        &configuration.enabled_rules,
        metadata,
        &mut diagnostics,
    );
    db_architecture::validate(
        root,
        &configuration.enabled_rules,
        metadata,
        &usable_exceptions,
        &mut diagnostics,
    );
    db_rls::validate(
        root,
        &configuration.enabled_rules,
        metadata,
        &mut diagnostics,
    );
    event_architecture::validate(root, &configuration.enabled_rules, &mut diagnostics);
    rpc_architecture::validate(root, &configuration.enabled_rules, &mut diagnostics);
    rust_architecture::validate(root, &configuration.enabled_rules, &mut diagnostics);
    diagnostics.sort();
    diagnostics
}

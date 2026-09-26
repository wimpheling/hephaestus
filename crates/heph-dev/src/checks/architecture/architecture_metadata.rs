use std::{collections::BTreeMap, path::Path};

use super::{CargoMetadata, CargoPackage, Diagnostic};

pub(super) fn validate_metadata(
    root: &Path,
    metadata: &CargoMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let metadata_root = metadata
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| metadata.workspace_root.clone());
    if canonical_root != metadata_root {
        diagnostics.push(Diagnostic::new(
            "ARCH-CARGO-METADATA",
            format!(
                "cargo metadata workspace root {} does not match repository root {}",
                metadata.workspace_root.display(),
                root.display()
            ),
        ));
    }

    let packages_by_id: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect();
    for member in &metadata.workspace_members {
        let Some(package) = packages_by_id.get(member.as_str()) else {
            diagnostics.push(Diagnostic::new(
                "ARCH-CARGO-METADATA",
                format!("workspace member {member} has no package record"),
            ));
            continue;
        };
        validate_package(root, package, diagnostics);
    }
}

pub(super) fn validate_package(
    root: &Path,
    package: &CargoPackage,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let manifest = package
        .manifest_path
        .canonicalize()
        .unwrap_or_else(|_| package.manifest_path.clone());
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if !manifest.starts_with(&canonical_root) || !manifest.is_file() {
        diagnostics.push(Diagnostic::new(
            "ARCH-CARGO-METADATA",
            format!(
                "workspace package {} has manifest outside the repository or missing: {}",
                package.name,
                package.manifest_path.display()
            ),
        ));
    }

    // Read these fields now so the stable metadata model cannot silently drift
    // before layer/dependency rules are activated by later constraints.
    let _architecture_declaration = package.metadata.get("hephaestus");
    for dependency in &package.dependencies {
        let Some(path) = &dependency.path else {
            continue;
        };
        let dependency_path = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !dependency_path.starts_with(&canonical_root) {
            diagnostics.push(Diagnostic::new(
                "ARCH-CARGO-METADATA",
                format!(
                    "workspace package {} has repository-escaping path dependency {} at {}",
                    package.name,
                    dependency.name,
                    path.display()
                ),
            ));
        }
    }
}

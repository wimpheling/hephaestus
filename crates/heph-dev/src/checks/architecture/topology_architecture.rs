//! Cargo metadata checks for the physical `heph-core`/`heph-std` boundary.

use super::{CargoMetadata, CargoPackage, Diagnostic};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const RULE: &str = "ARCH-CORE-NO-STD-DEPENDENCIES";

/// Reject every dependency edge from a package physically under `heph-core`
/// to a package physically under `heph-std`.
///
/// The classification deliberately uses canonical manifest directories. Cargo
/// package names are preserved during the topology migration and therefore do
/// not identify the physical ownership boundary.
pub(super) fn validate(
    root: &Path,
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
    let packages_by_path = packages
        .iter()
        .map(|package| (manifest_directory(package), *package))
        .collect::<BTreeMap<_, _>>();
    let core_prefix = canonical(&root.join("crates/heph-core"));
    let std_prefix = canonical(&root.join("crates/heph-std"));

    for source in packages {
        let source_path = manifest_directory(source);
        if !source_path.starts_with(&core_prefix) {
            continue;
        }
        for dependency in &source.dependencies {
            let Some(path) = dependency.path.as_ref() else {
                continue;
            };
            let Some(target) = packages_by_path.get(&canonical(path)) else {
                continue;
            };
            if !manifest_directory(target).starts_with(&std_prefix) {
                continue;
            }
            diagnostics.push(Diagnostic::new(
                RULE,
                format!(
                    "core package {} depends on standard package {} through a {} dependency; introduce a core port or move concrete wiring to the heph-app composition root",
                    source.name,
                    target.name,
                    dependency_kind(dependency.kind.as_deref()),
                ),
            ));
        }
    }
}

fn dependency_kind(kind: Option<&str>) -> &str {
    match kind {
        None => "normal",
        Some("dev") => "development",
        Some(kind) => kind,
    }
}

fn manifest_directory(package: &CargoPackage) -> PathBuf {
    canonical(
        package
            .manifest_path
            .parent()
            .unwrap_or(&package.manifest_path),
    )
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{RULE, validate};
    use crate::checks::architecture::{CargoDependency, CargoMetadata, CargoPackage, Diagnostic};
    use serde_json::json;
    use std::path::PathBuf;

    const ROOT: &str = "/workspace";

    fn package(
        id: &str,
        name: &str,
        relative_directory: &str,
        dependencies: Vec<CargoDependency>,
    ) -> CargoPackage {
        CargoPackage {
            id: id.to_owned(),
            name: name.to_owned(),
            manifest_path: PathBuf::from(ROOT)
                .join(relative_directory)
                .join("Cargo.toml"),
            metadata: json!({"hephaestus": {"layer": "adapter", "context": "fixture"}}),
            dependencies,
        }
    }

    fn dependency(name: &str, relative_directory: &str, kind: Option<&str>) -> CargoDependency {
        CargoDependency {
            name: name.to_owned(),
            path: Some(PathBuf::from(ROOT).join(relative_directory)),
            kind: kind.map(str::to_owned),
        }
    }

    fn metadata(packages: Vec<CargoPackage>) -> CargoMetadata {
        CargoMetadata {
            workspace_members: packages.iter().map(|package| package.id.clone()).collect(),
            packages,
            workspace_root: PathBuf::from(ROOT),
        }
    }

    fn diagnostics(packages: Vec<CargoPackage>) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        validate(
            &PathBuf::from(ROOT),
            &[RULE.to_owned()],
            &metadata(packages),
            &mut diagnostics,
        );
        diagnostics
    }

    #[test]
    fn valid_core_standard_and_composition_edges_have_no_diagnostics() {
        let core_contract = package(
            "core-contract-id",
            "std-looking-name",
            "crates/heph-core/platform",
            vec![],
        );
        let core_consumer = package(
            "core-consumer-id",
            "core-consumer",
            "crates/heph-core/identity/domain",
            vec![dependency(
                "std-looking-name",
                "crates/heph-core/platform",
                None,
            )],
        );
        let standard_provider = package(
            "standard-provider-id",
            "core-looking-name",
            "crates/heph-std/identity/oidc",
            vec![dependency(
                "std-looking-name",
                "crates/heph-core/platform",
                None,
            )],
        );
        let app = package(
            "app-id",
            "app",
            "crates/heph-app",
            vec![
                dependency("std-looking-name", "crates/heph-core/platform", None),
                dependency("core-looking-name", "crates/heph-std/identity/oidc", None),
            ],
        );

        assert!(diagnostics(vec![core_contract, core_consumer, standard_provider, app]).is_empty());
    }

    #[test]
    fn normal_build_and_development_core_to_standard_edges_are_rejected() {
        for (kind, expected) in [
            (None, "normal"),
            (Some("build"), "build"),
            (Some("dev"), "development"),
        ] {
            let source = package(
                &format!("source-{expected}"),
                &format!("source-{expected}"),
                "crates/heph-core/fixture/source",
                vec![dependency(
                    &format!("target-{expected}"),
                    "crates/heph-std/fixture/target",
                    kind,
                )],
            );
            let target = package(
                &format!("target-{expected}"),
                &format!("target-{expected}"),
                "crates/heph-std/fixture/target",
                vec![],
            );
            let diagnostics = diagnostics(vec![source, target]);
            assert_eq!(diagnostics.len(), 1, "dependency kind {expected}");
            assert!(
                diagnostics[0]
                    .message
                    .contains(&format!("source-{expected}"))
            );
            assert!(
                diagnostics[0]
                    .message
                    .contains(&format!("target-{expected}"))
            );
            assert!(
                diagnostics[0]
                    .message
                    .contains(&format!("{expected} dependency"))
            );
            assert!(diagnostics[0].message.contains("core port"));
            assert!(diagnostics[0].message.contains("heph-app composition root"));
        }
    }

    #[test]
    fn package_names_and_nearby_directory_prefixes_do_not_grant_or_trigger_classification() {
        let core_named_std = package(
            "heph-std",
            "heph-std",
            "crates/heph-core/fixture/core",
            vec![dependency(
                "core-named-std",
                "crates/heph-core/fixture/nearby",
                None,
            )],
        );
        let nearby_core = package(
            "core-named-std",
            "core-named-std",
            "crates/heph-core-extra/fixture/nearby",
            vec![],
        );
        let standard_named_core = package(
            "core",
            "core",
            "crates/heph-std-extra/fixture/standard",
            vec![],
        );

        assert!(diagnostics(vec![core_named_std, nearby_core, standard_named_core]).is_empty());
    }
}

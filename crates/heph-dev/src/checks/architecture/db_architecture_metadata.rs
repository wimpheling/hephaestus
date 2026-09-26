use super::{CargoMetadata, CargoPackage, Diagnostic, SQLX_RULE};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub(super) fn validate_metadata(
    metadata: &CargoMetadata,
    active: &BTreeSet<&str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !active.contains(SQLX_RULE) {
        return;
    }
    let workspace = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .collect::<Vec<_>>();
    let packages_by_root = workspace
        .iter()
        .map(|package| (manifest_root(package), *package))
        .collect::<BTreeMap<_, _>>();

    for package in &workspace {
        let declaration = adapter_declaration(package, diagnostics);
        if declaration {
            continue;
        }
        if has_dev_sqlx(package) && !has_test_only_sqlx(package) {
            diagnostics.push(Diagnostic::new(
                SQLX_RULE,
                format!(
                    "workspace package {} declares a dev-only SQLx test harness without `hephaestus.sqlx_test_dependency = true`",
                    package.name
                ),
            ));
        }
        let mut visited = BTreeSet::new();
        if let Some(path) = sqlx_path(package, &packages_by_root, &mut visited) {
            diagnostics.push(Diagnostic::new(
                SQLX_RULE,
                format!(
                    "workspace package {} reaches SQLx outside a declared PostgreSQL adapter: {}",
                    package.name,
                    path.join(" -> ")
                ),
            ));
        }
    }
}

fn adapter_declaration(package: &CargoPackage, diagnostics: &mut Vec<Diagnostic>) -> bool {
    let Some(hephaestus) = package.metadata.get("hephaestus") else {
        return false;
    };
    let Some(adapter) = hephaestus.get("postgres_adapter") else {
        return false;
    };
    if adapter != true {
        diagnostics.push(Diagnostic::new(
            SQLX_RULE,
            format!(
                "workspace package {} has non-boolean or false `hephaestus.postgres_adapter`; omit it or set it to true",
                package.name
            ),
        ));
        return false;
    }
    if !package.name.ends_with("-postgres") {
        diagnostics.push(Diagnostic::new(
            SQLX_RULE,
            format!(
                "PostgreSQL adapter {} must use a package name ending in `-postgres` when `hephaestus.postgres_adapter = true`",
                package.name
            ),
        ));
    }
    let valid_context = has_valid_database_context(package);
    if !valid_context {
        diagnostics.push(Diagnostic::new(
            SQLX_RULE,
            format!(
                "PostgreSQL adapter {} requires a non-empty lowercase `hephaestus.database_context`",
                package.name
            ),
        ));
    }
    package.name.ends_with("-postgres") && valid_context
}

fn manifest_root(package: &CargoPackage) -> PathBuf {
    package
        .manifest_path
        .parent()
        .unwrap_or(&package.manifest_path)
        .to_path_buf()
}

fn sqlx_path(
    package: &CargoPackage,
    packages_by_root: &BTreeMap<PathBuf, &CargoPackage>,
    visited: &mut BTreeSet<String>,
) -> Option<Vec<String>> {
    if !visited.insert(package.id.clone()) {
        return None;
    }
    for dependency in &package.dependencies {
        if dependency.name == "sqlx" && dependency.kind.as_deref() != Some("dev") {
            return Some(vec![package.name.clone(), String::from("sqlx")]);
        }
        if dependency.kind.as_deref() == Some("dev") {
            continue;
        }
        let Some(path) = dependency.path.as_ref() else {
            continue;
        };
        let Some(target) = packages_by_root.get(path) else {
            continue;
        };
        if is_declared_adapter(target) {
            continue;
        }
        if let Some(mut path) = sqlx_path(target, packages_by_root, visited) {
            path.insert(0, package.name.clone());
            return Some(path);
        }
    }
    None
}

/// Allows `SQLx` only for an explicitly declared test harness dependency.
fn has_test_only_sqlx(package: &CargoPackage) -> bool {
    let declared = package
        .metadata
        .get("hephaestus")
        .and_then(|metadata| metadata.get("sqlx_test_dependency"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    declared
        && package.dependencies.iter().any(|dependency| {
            dependency.name == "sqlx" && dependency.kind.as_deref() == Some("dev")
        })
}

fn has_dev_sqlx(package: &CargoPackage) -> bool {
    package
        .dependencies
        .iter()
        .any(|dependency| dependency.name == "sqlx" && dependency.kind.as_deref() == Some("dev"))
}

fn is_declared_adapter(package: &CargoPackage) -> bool {
    package.name.ends_with("-postgres")
        && package
            .metadata
            .get("hephaestus")
            .and_then(|metadata| metadata.get("postgres_adapter"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        && has_valid_database_context(package)
}

fn has_valid_database_context(package: &CargoPackage) -> bool {
    package
        .metadata
        .get("hephaestus")
        .and_then(|metadata| metadata.get("database_context"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|context| {
            !context.is_empty()
                && context
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

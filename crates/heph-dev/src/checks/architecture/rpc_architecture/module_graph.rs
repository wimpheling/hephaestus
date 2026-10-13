use std::{ffi::OsStr, fs, path::Path};

pub(super) fn is_test_only_source(root: &Path, path: &Path) -> bool {
    if is_package_integration_test(root, path) {
        return true;
    }
    is_test_only_source_inner(root, path, &mut Vec::new())
}

fn is_package_integration_test(root: &Path, path: &Path) -> bool {
    let mut directory = path.parent();
    while let Some(candidate) = directory {
        if !candidate.starts_with(root) {
            return false;
        }
        let manifest = candidate.join("Cargo.toml");
        if manifest.is_file() {
            let Ok(contents) = fs::read_to_string(manifest) else {
                return false;
            };
            return contents.lines().any(|line| line.trim() == "[package]")
                && path.starts_with(candidate.join("tests"));
        }
        if candidate == root {
            break;
        }
        directory = candidate.parent();
    }
    false
}

fn is_test_only_source_inner(root: &Path, path: &Path, seen: &mut Vec<std::path::PathBuf>) -> bool {
    if !path.starts_with(root) || seen.iter().any(|seen_path| seen_path == path) {
        return false;
    }
    seen.push(path.to_owned());
    for parent in module_parent_candidates(path) {
        if !parent.starts_with(root) || !parent.is_file() {
            continue;
        }
        let Ok(source) = fs::read_to_string(&parent) else {
            continue;
        };
        for (target, cfg_test) in module_targets(&parent, &source) {
            if target == path && (cfg_test || is_test_only_source_inner(root, &parent, seen)) {
                return true;
            }
        }
    }
    false
}

fn module_parent_candidates(path: &Path) -> Vec<std::path::PathBuf> {
    let Some(directory) = path.parent() else {
        return Vec::new();
    };
    let mut candidates = vec![directory.join("mod.rs")];
    if let (Some(parent), Some(name)) = (directory.parent(), directory.file_name()) {
        candidates.push(parent.join(format!("{}.rs", name.to_string_lossy())));
    }
    candidates
}

fn module_targets(source_path: &Path, source: &str) -> Vec<(std::path::PathBuf, bool)> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for item in file.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        if module.content.is_some() {
            continue;
        }
        let cfg_test = module.attrs.iter().any(|attribute| {
            attribute.path().is_ident("cfg")
                && attribute
                    .parse_args::<syn::Path>()
                    .is_ok_and(|condition| condition.is_ident("test"))
        });
        let mut path_attribute = None;
        let mut invalid_path = false;
        for attribute in &module.attrs {
            if !attribute.path().is_ident("path") {
                continue;
            }
            if path_attribute.is_some() {
                invalid_path = true;
                break;
            }
            if let syn::Meta::NameValue(value) = &attribute.meta
                && let syn::Expr::Lit(value) = &value.value
                && let syn::Lit::Str(value) = &value.lit
            {
                path_attribute = Some(value.value());
            } else {
                invalid_path = true;
                break;
            }
        }
        if invalid_path {
            continue;
        }
        let name = module.ident;
        let target = path_attribute.map_or_else(
            || {
                let parent = source_path.parent().unwrap_or_else(|| Path::new("."));
                if source_path.file_stem() == Some(OsStr::new("mod")) {
                    parent.join(format!("{name}.rs"))
                } else {
                    parent
                        .join(source_path.file_stem().unwrap_or_default())
                        .join(format!("{name}.rs"))
                }
            },
            |path| {
                source_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(path)
            },
        );
        targets.push((target, cfg_test));
    }
    targets
}

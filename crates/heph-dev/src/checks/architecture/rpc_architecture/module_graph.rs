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
    let mut targets = Vec::new();
    let mut cfg_test = false;
    let mut path_attribute = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]" {
            cfg_test = true;
            continue;
        }
        if let Some(value) = trimmed
            .strip_prefix("#[path = \"")
            .and_then(|value| value.strip_suffix("\"]"))
        {
            path_attribute = Some(value.to_owned());
            continue;
        }
        if trimmed.starts_with("#[") {
            continue;
        }
        let Some(module) = trimmed
            .strip_prefix("mod ")
            .or_else(|| trimmed.strip_prefix("pub mod "))
            .or_else(|| trimmed.strip_prefix("pub(crate) mod "))
            .and_then(|value| value.strip_suffix(';'))
        else {
            cfg_test = false;
            path_attribute = None;
            continue;
        };
        let Some(module) = module.split_whitespace().next() else {
            cfg_test = false;
            path_attribute = None;
            continue;
        };
        let target = path_attribute.take().map_or_else(
            || {
                let parent = source_path.parent().unwrap_or_else(|| Path::new("."));
                if source_path.file_stem() == Some(OsStr::new("mod")) {
                    parent.join(format!("{module}.rs"))
                } else {
                    parent
                        .join(source_path.file_stem().unwrap_or_default())
                        .join(format!("{module}.rs"))
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
        cfg_test = false;
    }
    targets
}

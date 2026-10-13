//! Read-only canonical path checks copied from accepted Invocation validation.

use super::{AppError, component, validation::invalid};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

pub(super) fn lexical(path: &Path) -> Result<(), AppError> {
    if !path.is_absolute()
        || path == Path::new("/")
        || path
            .to_str()
            .is_none_or(|value| value.len() > 4096 || value.chars().any(char::is_control))
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        || path.components().collect::<PathBuf>().as_os_str() != path.as_os_str()
    {
        return Err(invalid(
            "roots and sockets must be bounded canonical absolute paths",
        ));
    }
    Ok(())
}

pub(super) fn directory(path: &Path, private: bool, allow_missing: bool) -> Result<(), AppError> {
    lexical(path)?;
    let mut existing = path;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(metadata) => {
                let forbidden = if private && existing == path {
                    0o077
                } else {
                    0o022
                };
                if !metadata.is_dir()
                    || metadata.file_type().is_symlink()
                    || metadata.permissions().mode() & forbidden != 0
                    || metadata.uid() != rustix::process::geteuid().as_raw()
                    || fs::canonicalize(existing)
                        .map_err(component("managed Legacy canonical directory"))?
                        != existing
                {
                    return Err(invalid(
                        "initialized root or existing ancestor is unsafe, aliased or foreign",
                    ));
                }
                return Ok(());
            }
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .ok_or_else(|| invalid("root has no existing ancestor"))?;
            }
            Err(error) => return Err(component("managed Legacy directory metadata")(error)),
        }
    }
}

pub(super) fn allowed(path: &Path, roots: &[PathBuf]) -> Result<(), AppError> {
    let mut included = false;
    for root in roots {
        lexical(root)?;
        let metadata = fs::symlink_metadata(root)
            .map_err(component("managed Legacy provider allowed root"))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || fs::canonicalize(root)
                .map_err(component("managed Legacy canonical provider root"))?
                != *root
        {
            return Err(invalid(
                "actual provider allowed roots must be canonical non-symlink directories",
            ));
        }
        included |= path.starts_with(root);
    }
    if !included {
        return Err(invalid(
            "configured materialization path is outside actual provider allowed roots",
        ));
    }
    Ok(())
}

pub(super) fn executable(path: &Path) -> Result<(), AppError> {
    lexical(path)?;
    // Trusted mkfs.ext4 commonly names a system symlink to mke2fs. Inspect the
    // resolved executable without rewriting the configured invocation path.
    let metadata = fs::metadata(path).map_err(component("managed Legacy mkfs executable"))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(invalid(
            "mkfs.ext4 must resolve to an executable regular file",
        ));
    }
    Ok(())
}

pub(super) fn overlap(first: &Path, second: &Path) -> bool {
    first.starts_with(second) || second.starts_with(first)
}

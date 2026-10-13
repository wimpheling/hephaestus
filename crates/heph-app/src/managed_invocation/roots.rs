//! Canonical paths and provider root relationships, without creating directories.

use super::{AppError, ManagedInvocationInputs, RootFilesystem, component, validation::invalid};
use builder_catalog_domain::OciImageReference;
use heph_runtime::DiskFormat;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

pub(super) fn validate(inputs: &ManagedInvocationInputs) -> Result<(), AppError> {
    let actual = &inputs.expected_provider;
    let local = &inputs.local_volume;
    let roots = [
        local.volume_root.as_path(),
        actual.runtime_root.as_path(),
        actual.cgroup_root.as_path(),
        inputs.run_runtime.runtime_root.as_path(),
        inputs.run_runtime.release_artifact_root.as_path(),
        inputs.authority.root.as_path(),
        inputs.secret_mounts.root.as_path(),
    ];
    for (index, root) in roots.iter().enumerate() {
        lexical(root)?;
        if roots[index + 1..].iter().any(|other| overlap(root, other)) {
            return Err(invalid(
                "volume, VM/cgroup, runtime, release, authority and secret roots must not overlap",
            ));
        }
    }
    directory(&inputs.run_runtime.runtime_root, false, true)?;
    directory(&inputs.run_runtime.release_artifact_root, false, true)?;
    directory(&inputs.authority.root, true, true)?;
    directory(&inputs.secret_mounts.root, true, false)?;
    for transient in &local.transient_runtime_roots {
        lexical(transient)?;
        if overlap(transient, &local.volume_root) {
            return Err(invalid(
                "persistent volume root overlaps a configured transient root",
            ));
        }
    }
    if !local
        .transient_runtime_roots
        .iter()
        .any(|root| actual.runtime_root.starts_with(root))
    {
        return Err(invalid(
            "local volume transient roots must cover the actual VM runtime root",
        ));
    }
    allowed(&local.volume_root, &actual.disk_roots)?;
    allowed(&inputs.run_runtime.runtime_root, &actual.mount_roots)?;
    allowed(&inputs.secret_mounts.root, &actual.mount_roots)?;
    executable(&local.mkfs_ext4)?;
    for socket in [&actual.broker_socket_path, &actual.runtime_git_socket_path]
        .into_iter()
        .flatten()
    {
        lexical(socket)?;
        if roots.iter().any(|root| socket.starts_with(root)) {
            return Err(invalid(
                "provider broker/Git sockets must be outside initialized and managed roots",
            ));
        }
    }
    validate_images(inputs)
}

fn validate_images(inputs: &ManagedInvocationInputs) -> Result<(), AppError> {
    if inputs.root_images.is_empty() {
        return Err(invalid("at least one root image mapping is required"));
    }
    for (reference, root) in &inputs.root_images {
        // Same digest-pinned parser as the existing private App validator.
        OciImageReference::parse(reference.clone()).map_err(|error| {
            AppError::Configuration(format!(
                "root image reference {reference:?} is not digest-pinned: {error}"
            ))
        })?;
        let (path, expected_directory) = match root {
            RootFilesystem::Directory { host_path } => (host_path, true),
            RootFilesystem::Disk {
                host_path,
                format: DiskFormat::Raw,
                ..
            } => (host_path, false),
            _ => {
                return Err(invalid(
                    "root image uses an unsupported filesystem variant/format",
                ));
            }
        };
        lexical(path)?;
        let metadata = fs::symlink_metadata(path)
            .map_err(component("qualified Invocation root image metadata"))?;
        if metadata.file_type().is_symlink()
            || (expected_directory && !metadata.is_dir())
            || (!expected_directory && !metadata.is_file())
            || fs::canonicalize(path)
                .map_err(component("qualified Invocation canonical root image"))?
                != *path
        {
            return Err(invalid(
                "root image must be the canonical non-symlink directory or raw regular file",
            ));
        }
        allowed(path, &inputs.expected_provider.image_roots)?;
        if [
            &inputs.local_volume.volume_root,
            &inputs.expected_provider.runtime_root,
            &inputs.expected_provider.cgroup_root,
            &inputs.run_runtime.runtime_root,
            &inputs.run_runtime.release_artifact_root,
            &inputs.authority.root,
            &inputs.secret_mounts.root,
        ]
        .iter()
        .any(|mutable| overlap(path, mutable))
        {
            return Err(invalid(
                "root image materialization overlaps managed mutable roots",
            ));
        }
    }
    Ok(())
}

fn lexical(path: &Path) -> Result<(), AppError> {
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

fn directory(path: &Path, private: bool, allow_missing: bool) -> Result<(), AppError> {
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
                        .map_err(component("qualified Invocation canonical directory"))?
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
            Err(error) => return Err(component("qualified Invocation directory metadata")(error)),
        }
    }
}

fn allowed(path: &Path, roots: &[PathBuf]) -> Result<(), AppError> {
    let mut included = false;
    for root in roots {
        lexical(root)?;
        let metadata = fs::symlink_metadata(root)
            .map_err(component("qualified Invocation provider allowed root"))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || fs::canonicalize(root)
                .map_err(component("qualified Invocation canonical provider root"))?
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

fn executable(path: &Path) -> Result<(), AppError> {
    lexical(path)?;
    // Trusted mkfs.ext4 commonly names a system symlink to mke2fs. Inspect the
    // resolved executable without rewriting the configured invocation path.
    let metadata = fs::metadata(path).map_err(component("qualified Invocation mkfs executable"))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(invalid(
            "mkfs.ext4 must resolve to an executable regular file",
        ));
    }
    Ok(())
}

fn overlap(first: &Path, second: &Path) -> bool {
    first.starts_with(second) || second.starts_with(first)
}

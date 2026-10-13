//! Canonical paths and provider root relationships, without creating directories.

use super::paths::{allowed, directory, executable, lexical, overlap};
use super::{AppError, ManagedLegacyInputs, RootFilesystem, component, validation::invalid};
use builder_catalog_domain::OciImageReference;
use heph_runtime::DiskFormat;
use std::fs;

pub(super) fn validate(inputs: &ManagedLegacyInputs) -> Result<(), AppError> {
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
        inputs.workspaces.workspace_root.as_path(),
        inputs.workspaces.artifact_root.as_path(),
        inputs.workspaces.repository_root.as_path(),
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

fn validate_images(inputs: &ManagedLegacyInputs) -> Result<(), AppError> {
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
        let metadata =
            fs::symlink_metadata(path).map_err(component("managed Legacy root image metadata"))?;
        if metadata.file_type().is_symlink()
            || (expected_directory && !metadata.is_dir())
            || (!expected_directory && !metadata.is_file())
            || fs::canonicalize(path).map_err(component("managed Legacy canonical root image"))?
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
            &inputs.workspaces.workspace_root,
            &inputs.workspaces.artifact_root,
            &inputs.workspaces.repository_root,
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

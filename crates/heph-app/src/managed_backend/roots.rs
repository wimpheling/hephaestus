//! Dedicated mutable paths, checked without filesystem initialization.

use super::{AppConfig, AppError, ManagedBackendConfig, custody, invalid, paths};
use crate::{RootFilesystem, VmBackendConfig};
use heph_runtime::DiskFormat;
use std::{fs, path::Path};

pub(super) fn validate(app: &AppConfig, config: &ManagedBackendConfig) -> Result<(), AppError> {
    let roots = mutable_roots(config);
    for (index, root) in roots.iter().enumerate() {
        paths::lexical(root)?;
        for other in &roots[index + 1..] {
            disjoint(root, other)?;
        }
    }
    for path in [
        &config.run_runtime.runtime_root,
        &config.run_runtime.release_artifact_root,
        &config.workspaces.workspace_root,
        &config.workspaces.artifact_root,
    ] {
        paths::directory(path, false, true)?;
    }
    paths::directory(&config.authority_root, true, true)?;
    paths::directory(&config.secret_mounts.root, true, false)?;
    paths::directory(&config.workspaces.repository_root, false, false)?;
    paths::executable(&config.workspaces.git_binary)?;
    paths::executable(&config.volumes.mkfs_ext4)?;
    if config.workspaces.limits.max_file_bytes > config.workspaces.limits.max_total_bytes {
        return Err(invalid("workspace per-file quota exceeds aggregate quota"));
    }
    for (path, allowed) in [
        (&config.volumes.volume_root, &config.provider.disk_roots),
        (
            &config.run_runtime.runtime_root,
            &config.provider.mount_roots,
        ),
        (&config.secret_mounts.root, &config.provider.mount_roots),
        (
            &config.workspaces.workspace_root,
            &config.provider.mount_roots,
        ),
    ] {
        paths::allowed(path, allowed)?;
    }
    for transient in &config.volumes.transient_runtime_roots {
        paths::lexical(transient)?;
        disjoint(transient, &config.volumes.volume_root)?;
    }
    if !config
        .volumes
        .transient_runtime_roots
        .iter()
        .any(|path| config.provider.runtime_root.starts_with(path))
    {
        return Err(invalid(
            "managed volume transient roots must cover the dedicated VM root",
        ));
    }
    separation(app, config, &roots)?;
    for socket in [&config.runtime_git_socket, &app.secret_broker_socket] {
        paths::lexical(socket)?;
        for root in &roots {
            disjoint(socket, root)?;
        }
    }
    images(app, config, &roots)
}

fn mutable_roots(config: &ManagedBackendConfig) -> [&Path; 9] {
    [
        config.volumes.volume_root.as_path(),
        config.provider.runtime_root.as_path(),
        config.provider.cgroup_root.as_path(),
        config.run_runtime.runtime_root.as_path(),
        config.run_runtime.release_artifact_root.as_path(),
        config.authority_root.as_path(),
        config.secret_mounts.root.as_path(),
        config.workspaces.workspace_root.as_path(),
        config.workspaces.artifact_root.as_path(),
    ]
}

fn separation(
    app: &AppConfig,
    config: &ManagedBackendConfig,
    managed: &[&Path],
) -> Result<(), AppError> {
    let mut ordinary = vec![
        app.volumes.volume_root.as_path(),
        app.workspaces.workspace_root.as_path(),
        app.workspaces.artifact_root.as_path(),
        app.run_runtime.runtime_root.as_path(),
        app.run_runtime.release_artifact_root.as_path(),
        app.runtime_authority_handoff_root.as_path(),
        app.secret_mounts.root.as_path(),
        app.build_workspace_root.as_path(),
        app.repository_root.as_path(),
    ];
    ordinary.extend(
        app.volumes
            .transient_runtime_roots
            .iter()
            .map(std::path::PathBuf::as_path),
    );
    if let VmBackendConfig::Libkrun(provider) = &app.vm_backend {
        ordinary.extend([
            provider.runtime_root.as_path(),
            provider.cgroup_root.as_path(),
        ]);
        ordinary.extend(
            provider
                .disk_roots
                .iter()
                .chain(&provider.mount_roots)
                .map(std::path::PathBuf::as_path),
        );
    }
    if let Some(oci) = &app.oci_builder {
        ordinary.extend([
            oci.runtime.checkout_root.as_path(),
            oci.runtime.output_root.as_path(),
            oci.verification_root.as_path(),
            oci.scratch_root.as_path(),
            oci.rootfs_root.as_path(),
            oci.root_manifest.as_path(),
        ]);
        ordinary.extend(
            oci.runtime
                .verified_rootfs_root
                .iter()
                .map(std::path::PathBuf::as_path),
        );
        ordinary.extend(
            oci.runtime
                .image_layouts
                .values()
                .map(std::path::PathBuf::as_path),
        );
    }
    // Allowed writable capabilities are included too: a broad managed mount/disk
    // root must not grant the dedicated node access to ordinary mutable trees.
    let mut dedicated = managed.to_vec();
    dedicated.extend(
        config
            .provider
            .disk_roots
            .iter()
            .chain(&config.provider.mount_roots)
            .map(std::path::PathBuf::as_path),
    );
    for root in &ordinary {
        paths::lexical(root)?;
        // Resolve the existing ancestor as well, denying symlink aliases even
        // when this ordinary initializer would create a missing descendant.
        canonical_ancestor(root)?;
        if app
            .oci_builder
            .as_ref()
            .is_none_or(|oci| root != &oci.root_manifest.as_path())
        {
            paths::directory(root, false, true)?;
        }
        for other in &dedicated {
            disjoint(root, other)?;
        }
    }
    for root in &dedicated {
        paths::lexical(root)?;
        canonical_ancestor(root)?;
    }
    Ok(())
}

fn canonical_ancestor(path: &Path) -> Result<(), AppError> {
    let mut existing = path;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || fs::canonicalize(existing).map_err(custody::failure)? != existing
                {
                    return Err(invalid("mutable path aliases an existing physical root"));
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .ok_or_else(|| invalid("mutable root has no existing ancestor"))?;
            }
            Err(error) => return Err(custody::failure(error)),
        }
    }
}

fn images(app: &AppConfig, config: &ManagedBackendConfig, roots: &[&Path]) -> Result<(), AppError> {
    if app.root_images.is_empty() {
        return Err(invalid("managed root image mappings are required"));
    }
    for (reference, root) in &app.root_images {
        builder_catalog_domain::OciImageReference::parse(reference.clone())
            .map_err(custody::failure)?;
        let (path, directory) = match root {
            RootFilesystem::Directory { host_path } => (host_path, true),
            RootFilesystem::Disk {
                host_path,
                format: DiskFormat::Raw,
                ..
            } => (host_path, false),
            _ => {
                return Err(invalid(
                    "managed root images require a directory or raw disk",
                ));
            }
        };
        paths::lexical(path)?;
        let metadata = fs::symlink_metadata(path).map_err(custody::failure)?;
        if metadata.file_type().is_symlink()
            || (directory && !metadata.is_dir())
            || (!directory && !metadata.is_file())
            || fs::canonicalize(path).map_err(custody::failure)? != *path
        {
            return Err(invalid(
                "managed root image has an unsafe or noncanonical shape",
            ));
        }
        paths::allowed(path, &config.provider.image_roots)?;
        for mutable in roots {
            disjoint(path, mutable)?;
        }
        disjoint(path, &config.workspaces.repository_root)?;
    }
    Ok(())
}

pub(super) fn disjoint(first: &Path, second: &Path) -> Result<(), AppError> {
    if paths::overlap(first, second) {
        return Err(invalid(
            "ordinary and managed mutable roots must not overlap",
        ));
    }
    Ok(())
}

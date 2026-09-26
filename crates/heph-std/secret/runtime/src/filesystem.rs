//! Safe root validation, cleanup, reconciliation, and provider adapter.

use secret_domain::OpaqueRuntimeCredential;
use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::Path};
use uuid::Uuid;

use runtime_types::RunId;

use super::{
    EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretMountProvider,
    SecretRuntimeError,
};

pub fn validate_root(config: &EphemeralSecretConfig) -> Result<(), SecretRuntimeError> {
    let metadata = fs::symlink_metadata(&config.root).map_err(io_error)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(SecretRuntimeError::InvalidRoot);
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(SecretRuntimeError::InvalidRootPermissions);
    }
    if config.require_memory_filesystem && !is_memory_backed(&config.root)? {
        return Err(SecretRuntimeError::NotMemoryBacked);
    }
    Ok(())
}

pub fn is_memory_backed(path: &Path) -> Result<bool, SecretRuntimeError> {
    let canonical = fs::canonicalize(path).map_err(io_error)?;
    let mount_info = fs::read_to_string("/proc/self/mountinfo").map_err(io_error)?;
    let mut best: Option<(usize, &str)> = None;
    for line in mount_info.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let Some(mount_path) = left.split_whitespace().nth(4) else {
            continue;
        };
        let Some(filesystem) = right.split_whitespace().next() else {
            continue;
        };
        let mount = Path::new(mount_path);
        if canonical.starts_with(mount) && best.is_none_or(|(length, _)| mount_path.len() > length)
        {
            best = Some((mount_path.len(), filesystem));
        }
    }
    Ok(best.is_some_and(|(_, filesystem)| matches!(filesystem, "tmpfs" | "ramfs")))
}

/// Removes opaque orphan directories not associated with a live runtime.
///
/// # Errors
///
/// Rejects malformed names, symlinks, special files, and I/O failures instead
/// of traversing an attacker-controlled path.
pub fn reconcile_orphans(
    config: &EphemeralSecretConfig,
    live_directories: &BTreeSet<String>,
) -> Result<usize, SecretRuntimeError> {
    validate_root(config)?;
    let mut removed = 0;
    for entry in fs::read_dir(&config.root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| SecretRuntimeError::InvalidOrphan)?;
        if Uuid::parse_str(&name).is_err() {
            return Err(SecretRuntimeError::InvalidOrphan);
        }
        if live_directories.contains(&name) {
            continue;
        }
        remove_secret_directory(&entry.path())?;
        removed += 1;
    }
    Ok(removed)
}

/// Removes one persisted opaque mount after provider cleanup is confirmed.
///
/// # Errors
///
/// Rejects an unsafe root, symlink, or special filesystem object.
pub fn destroy_confirmed(
    config: &EphemeralSecretConfig,
    opaque_directory: Uuid,
) -> Result<(), SecretRuntimeError> {
    validate_root(config)?;
    let path = config.root.join(opaque_directory.simple().to_string());
    match fs::symlink_metadata(&path) {
        Ok(_) => remove_secret_directory(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

/// Standard filesystem implementation of the secret mount provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilesystemSecretMountProvider;

impl FilesystemSecretMountProvider {
    /// Creates the filesystem provider.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl SecretMountProvider for FilesystemSecretMountProvider {
    fn validate_config(&self, config: &EphemeralSecretConfig) -> Result<(), SecretRuntimeError> {
        validate_root(config)
    }

    fn materialize(
        &self,
        config: &EphemeralSecretConfig,
        run_id: RunId,
        files: Vec<RawSecretFile>,
        credential: Option<&OpaqueRuntimeCredential>,
    ) -> Result<MaterializedSecretMount, SecretRuntimeError> {
        let mount = match credential {
            Some(credential) => {
                super::materialize::materialize_with_authority(config, run_id, files, credential)?
            }
            None => super::materialize::materialize(config, run_id, files)?,
        };
        let opaque_directory = mount
            .host_path()
            .file_name()
            .and_then(|value| value.to_str())
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(SecretRuntimeError::InvalidOrphan)?;
        Ok(MaterializedSecretMount {
            opaque_directory,
            vm_mount: mount.vm_mount(),
        })
    }

    fn discard_materialized(
        &self,
        config: &EphemeralSecretConfig,
        opaque_directory: Uuid,
    ) -> Result<(), SecretRuntimeError> {
        destroy_confirmed(config, opaque_directory)
    }

    fn destroy_confirmed(
        &self,
        config: &EphemeralSecretConfig,
        opaque_directory: Uuid,
    ) -> Result<(), SecretRuntimeError> {
        destroy_confirmed(config, opaque_directory)
    }

    fn reconcile_orphans(
        &self,
        config: &EphemeralSecretConfig,
        live_directories: &BTreeSet<String>,
    ) -> Result<usize, SecretRuntimeError> {
        reconcile_orphans(config, live_directories)
    }
}

pub fn remove_secret_directory(path: &Path) -> Result<(), SecretRuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(SecretRuntimeError::UnsafeObject);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(SecretRuntimeError::UnsafeObject);
        }
        fs::remove_file(entry.path()).map_err(io_error)?;
    }
    fs::remove_dir(path).map_err(io_error)
}

// `Result::map_err` supplies an owned error, so this adapter intentionally
// matches that callable signature while retaining only the redacted kind.
#[allow(clippy::needless_pass_by_value)]
pub fn io_error(error: std::io::Error) -> SecretRuntimeError {
    SecretRuntimeError::Io(error.kind())
}

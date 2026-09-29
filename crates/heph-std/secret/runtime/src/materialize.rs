//! Bounded raw-secret materialization.

use secret_domain::OpaqueRuntimeCredential;
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};
use uuid::Uuid;

use runtime_types::RunId;

use super::filesystem::{io_error, remove_secret_directory, validate_root};
use super::{
    EphemeralSecretConfig, EphemeralSecretMount, MAX_RAW_SECRET_BYTES, MAX_RAW_SECRET_FILES,
    RUNTIME_CREDENTIAL_FILE, RawSecretFile, SecretMountState, SecretRuntimeError,
};

/// Materializes bounded secret values into one opaque directory.
///
/// # Errors
///
/// Rejects a non-memory-backed root when required, bounds violations,
/// duplicate slots, existing/symlink paths, unsafe filesystem objects, and
/// I/O failures.
pub fn materialize(
    config: &EphemeralSecretConfig,
    run_id: RunId,
    files: Vec<RawSecretFile>,
) -> Result<EphemeralSecretMount, SecretRuntimeError> {
    materialize_inner(config, run_id, files, None)
}

/// Materializes raw values and the short-lived opaque runtime credential.
///
/// The credential permits live broker or raw-lease authentication but is not
/// itself a durable secret version. It is protected and cleaned with the same
/// strict guest-lifecycle ordering as raw values.
///
/// # Errors
///
/// Returns the same bounded, non-disclosing errors as [`materialize`].
pub fn materialize_with_authority(
    config: &EphemeralSecretConfig,
    run_id: RunId,
    files: Vec<RawSecretFile>,
    credential: &OpaqueRuntimeCredential,
) -> Result<EphemeralSecretMount, SecretRuntimeError> {
    materialize_inner(config, run_id, files, Some(credential))
}

fn materialize_inner(
    config: &EphemeralSecretConfig,
    run_id: RunId,
    files: Vec<RawSecretFile>,
    credential: Option<&OpaqueRuntimeCredential>,
) -> Result<EphemeralSecretMount, SecretRuntimeError> {
    validate_root(config)?;
    if (files.is_empty() && credential.is_none()) || files.len() > MAX_RAW_SECRET_FILES {
        return Err(SecretRuntimeError::FileCount);
    }
    let total = files.iter().try_fold(0_usize, |total, file| {
        total
            .checked_add(file.value.len())
            .ok_or(SecretRuntimeError::TotalSize)
    })?;
    if total > MAX_RAW_SECRET_BYTES {
        return Err(SecretRuntimeError::TotalSize);
    }
    let mut slots = BTreeSet::new();
    if files
        .iter()
        .any(|file| !slots.insert(file.slot.as_str().to_owned()))
    {
        return Err(SecretRuntimeError::DuplicateSlot);
    }

    let host_path = config.root.join(Uuid::new_v4().simple().to_string());
    fs::create_dir(&host_path).map_err(io_error)?;
    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    let result = write_files(&host_path, &files, credential);
    if let Err(error) = result {
        let _cleanup = remove_secret_directory(&host_path);
        return Err(error);
    }
    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o500)).map_err(io_error)?;
    Ok(EphemeralSecretMount {
        run_id,
        host_path,
        slots: files.into_iter().map(|file| file.slot).collect(),
        state: SecretMountState::Materialized,
    })
}

fn write_files(
    directory: &Path,
    files: &[RawSecretFile],
    credential: Option<&OpaqueRuntimeCredential>,
) -> Result<(), SecretRuntimeError> {
    for file in files {
        let path = directory.join(file.slot.as_str());
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .custom_flags(libc_flags())
            .open(&path)
            .map_err(io_error)?;
        output.write_all(file.value.expose()).map_err(io_error)?;
        output.flush().map_err(io_error)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o400)).map_err(io_error)?;
    }
    if let Some(credential) = credential {
        let path = directory.join(RUNTIME_CREDENTIAL_FILE);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .custom_flags(libc_flags())
            .open(&path)
            .map_err(io_error)?;
        output.write_all(credential.expose()).map_err(io_error)?;
        output.flush().map_err(io_error)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o400)).map_err(io_error)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
const fn libc_flags() -> i32 {
    // Linux O_NOFOLLOW and O_CLOEXEC values are stable ABI constants.
    0o400_000 | 0o2_000_000
}

#[cfg(not(target_os = "linux"))]
const fn libc_flags() -> i32 {
    0
}

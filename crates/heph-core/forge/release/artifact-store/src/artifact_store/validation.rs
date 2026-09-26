use release_domain::ArtifactPath;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;

use super::{ArtifactStoreError, MAX_ARTIFACT_FILES, errors::io_error, hashing::hash_file};

pub fn stable_storage_key(operation_id: Uuid, relative: &str, hash: &[u8; 32]) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus.release-artifact.v1");
    digest.update(operation_id.as_bytes());
    digest.update((relative.len() as u64).to_be_bytes());
    digest.update(relative.as_bytes());
    digest.update(hash);
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 version 8 marks this as an application-defined opaque UUID.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

pub fn validate_existing_object(
    path: &Path,
    expected_hash: &[u8; 32],
    expected_length: u64,
) -> Result<(), ArtifactStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.nlink() != 1
        || metadata.len() != expected_length
    {
        return Err(ArtifactStoreError::ObjectConflict);
    }
    let (hash, length) = hash_file(path)?;
    if hash != *expected_hash || length != expected_length {
        return Err(ArtifactStoreError::ObjectConflict);
    }
    Ok(())
}

pub fn validate_source_root(path: &Path) -> Result<(), ArtifactStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(ArtifactStoreError::InvalidSourceRoot);
    }
    Ok(())
}

pub fn discover(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(String, PathBuf, fs::Metadata)>,
) -> Result<(), ArtifactStoreError> {
    for entry in fs::read_dir(directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(ArtifactStoreError::UnsafeObject);
        }
        if file_type.is_dir() {
            discover(root, &path, output)?;
        } else if file_type.is_file() {
            if metadata.nlink() != 1 {
                return Err(ArtifactStoreError::HardLink);
            }
            if metadata.permissions().mode() & 0o7000 != 0 {
                return Err(ArtifactStoreError::UnsafeMode);
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| ArtifactStoreError::InvalidPath)?
                .to_str()
                .ok_or(ArtifactStoreError::InvalidPath)?
                .replace(std::path::MAIN_SEPARATOR, "/");
            ArtifactPath::parse(relative.clone()).map_err(|_| ArtifactStoreError::InvalidPath)?;
            output.push((relative, path, metadata));
        } else {
            return Err(ArtifactStoreError::UnsafeObject);
        }
        if output.len() > MAX_ARTIFACT_FILES {
            return Err(ArtifactStoreError::FileCount);
        }
    }
    Ok(())
}

use release_domain::{ArtifactKind, ArtifactPath, ContentHash};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;

use super::{
    ArtifactStoreError, MAX_ARTIFACT_BYTES, MAX_ARTIFACT_FILES,
    errors::io_error,
    hashing::copy_and_hash,
    validation::{discover, stable_storage_key, validate_existing_object, validate_source_root},
};

/// One immutable safely imported artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedArtifact {
    /// Release-relative normalized path.
    pub path: ArtifactPath,
    /// Inferred executable or ordinary file kind.
    pub kind: ArtifactKind,
    /// Normalized immutable Unix mode.
    pub mode: u16,
    /// Hash of exact bytes.
    pub content_hash: ContentHash,
    /// Exact length.
    pub size_bytes: u64,
    /// Opaque host storage identity.
    pub storage_key: Uuid,
}

/// Local canonical artifact store.
#[derive(Debug, Clone)]
pub struct LocalArtifactStore {
    root: PathBuf,
}

impl LocalArtifactStore {
    /// Opens an administrator-owned store root.
    ///
    /// # Errors
    ///
    /// Rejects missing, symlink, non-directory, or group/world-writable roots.
    pub fn new(root: PathBuf) -> Result<Self, ArtifactStoreError> {
        let metadata = fs::symlink_metadata(&root).map_err(io_error)?;
        if !metadata.file_type().is_dir()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(ArtifactStoreError::InvalidStoreRoot);
        }
        Ok(Self { root })
    }

    /// Imports a sealed output tree once and returns a deterministic manifest.
    ///
    /// # Errors
    ///
    /// Rejects symlinks, hard links, devices, sockets, FIFOs, path escapes,
    /// file/count bounds, mutation while reading, and host I/O failures.
    pub fn import(
        &self,
        sealed_output: &Path,
    ) -> Result<Vec<ImportedArtifact>, ArtifactStoreError> {
        self.import_inner(sealed_output, None)
    }

    /// Imports a sealed tree using stable opaque storage identities.
    ///
    /// Repeating an import for the same durable operation and identical tree
    /// returns the same manifest. Existing canonical objects are verified
    /// byte-for-byte and are never overwritten.
    ///
    /// # Errors
    ///
    /// Returns the same safe-import failures as [`Self::import`], and rejects
    /// any conflicting object already stored under a derived identity.
    pub fn import_for(
        &self,
        operation_id: Uuid,
        sealed_output: &Path,
    ) -> Result<Vec<ImportedArtifact>, ArtifactStoreError> {
        self.import_inner(sealed_output, Some(operation_id))
    }

    fn import_inner(
        &self,
        sealed_output: &Path,
        operation_id: Option<Uuid>,
    ) -> Result<Vec<ImportedArtifact>, ArtifactStoreError> {
        validate_source_root(sealed_output)?;
        let mut discovered = Vec::new();
        discover(sealed_output, sealed_output, &mut discovered)?;
        discovered.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        if discovered.is_empty() || discovered.len() > MAX_ARTIFACT_FILES {
            return Err(ArtifactStoreError::FileCount);
        }
        let total = discovered
            .iter()
            .try_fold(0_u64, |total, (_, _, metadata)| {
                total
                    .checked_add(metadata.len())
                    .ok_or(ArtifactStoreError::TotalSize)
            })?;
        if total > MAX_ARTIFACT_BYTES {
            return Err(ArtifactStoreError::TotalSize);
        }
        let transaction = self.root.join(format!(".import-{}", Uuid::new_v4()));
        fs::create_dir(&transaction).map_err(io_error)?;
        fs::set_permissions(&transaction, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        let result = self.copy_all(&transaction, discovered, operation_id);
        if result.is_err() {
            let _cleanup = fs::remove_dir_all(&transaction);
        }
        result
    }

    fn copy_all(
        &self,
        transaction: &Path,
        discovered: Vec<(String, PathBuf, fs::Metadata)>,
        operation_id: Option<Uuid>,
    ) -> Result<Vec<ImportedArtifact>, ArtifactStoreError> {
        let mut manifest = Vec::with_capacity(discovered.len());
        for (relative, source, before) in discovered {
            let staged = transaction.join(Uuid::new_v4().simple().to_string());
            let (hash, length) = copy_and_hash(&source, &staged)?;
            let after = fs::symlink_metadata(&source).map_err(io_error)?;
            if before.dev() != after.dev()
                || before.ino() != after.ino()
                || before.len() != after.len()
                || before.mtime() != after.mtime()
                || before.mtime_nsec() != after.mtime_nsec()
            {
                return Err(ArtifactStoreError::SourceChanged);
            }
            let storage_key = operation_id.map_or_else(Uuid::new_v4, |identity| {
                stable_storage_key(identity, &relative, &hash)
            });
            let canonical = self.root.join(storage_key.simple().to_string());
            match fs::hard_link(&staged, &canonical) {
                Ok(()) => {
                    fs::remove_file(&staged).map_err(io_error)?;
                    fs::set_permissions(&canonical, fs::Permissions::from_mode(0o400))
                        .map_err(io_error)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    validate_existing_object(&canonical, &hash, length)?;
                    fs::remove_file(&staged).map_err(io_error)?;
                }
                Err(error) => return Err(io_error(error)),
            }
            let executable = before.permissions().mode() & 0o111 != 0;
            manifest.push(ImportedArtifact {
                path: ArtifactPath::parse(relative).map_err(|_| ArtifactStoreError::InvalidPath)?,
                kind: if executable {
                    ArtifactKind::Executable
                } else {
                    ArtifactKind::File
                },
                mode: if executable { 0o555 } else { 0o444 },
                content_hash: ContentHash::from_digest(hash),
                size_bytes: length,
                storage_key,
            });
        }
        fs::remove_dir(transaction).map_err(io_error)?;
        Ok(manifest)
    }

    /// Resolves an opaque storage identity without accepting a tenant path.
    ///
    /// # Errors
    ///
    /// Rejects missing, symlink, or non-regular objects.
    pub fn resolve(&self, storage_key: Uuid) -> Result<PathBuf, ArtifactStoreError> {
        let path = self.root.join(storage_key.simple().to_string());
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(ArtifactStoreError::UnsafeObject);
        }
        Ok(path)
    }

    /// Reads one opaque object only after verifying its complete contents.
    ///
    /// The object is opened once with no-follow semantics, bounded by the
    /// caller's limit, and hashed before any bytes are returned. The returned
    /// buffer therefore cannot refer to a path that was swapped after a
    /// metadata check or contain bytes that were not covered by verification.
    ///
    /// # Errors
    ///
    /// Rejects objects over `max_bytes`, missing or unsafe objects, length or
    /// hash mismatches, and filesystem failures.
    pub fn read_verified(
        &self,
        storage_key: Uuid,
        expected_hash: ContentHash,
        expected_size: u64,
        max_bytes: u64,
    ) -> Result<Vec<u8>, ArtifactStoreError> {
        if expected_size > max_bytes {
            return Err(ArtifactStoreError::ReadLimit);
        }
        let path = self.root.join(storage_key.simple().to_string());
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(io_error)?;
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.file_type().is_file() || metadata.nlink() != 1 {
            return Err(ArtifactStoreError::UnsafeObject);
        }
        if metadata.len() != expected_size {
            return Err(ArtifactStoreError::ObjectConflict);
        }
        let expected_length =
            usize::try_from(expected_size).map_err(|_| ArtifactStoreError::ReadLimit)?;
        let capacity = expected_length
            .checked_add(1)
            .ok_or(ArtifactStoreError::ReadLimit)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| ArtifactStoreError::ReadLimit)?;
        file.take(expected_size.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() != expected_length
            || Sha256::digest(&bytes).as_slice() != expected_hash.as_bytes()
        {
            return Err(ArtifactStoreError::ObjectConflict);
        }
        Ok(bytes)
    }
}

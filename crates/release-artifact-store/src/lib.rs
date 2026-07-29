//! Safe one-way import from an untrusted sealed build-output directory.
//!
//! Import accepts only ordinary directories and single-linked regular files,
//! hashes bytes while copying into an opaque immutable store, and returns a
//! deterministic path-sorted manifest. Repository-controlled paths never
//! select canonical host storage locations.

use release_domain::{ArtifactKind, ArtifactPath, ContentHash};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Maximum files in one release import.
pub const MAX_ARTIFACT_FILES: usize = 4_096;
/// Maximum aggregate imported bytes.
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

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
}

fn stable_storage_key(operation_id: Uuid, relative: &str, hash: &[u8; 32]) -> Uuid {
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

fn validate_existing_object(
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

fn validate_source_root(path: &Path) -> Result<(), ArtifactStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(ArtifactStoreError::InvalidSourceRoot);
    }
    Ok(())
}

fn discover(
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

fn copy_and_hash(source: &Path, destination: &Path) -> Result<([u8; 32], u64), ArtifactStoreError> {
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(o_nofollow())
        .open(source)
        .map_err(io_error)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(o_nofollow())
        .open(destination)
        .map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(count).map_err(|_| ArtifactStoreError::TotalSize)?)
            .ok_or(ArtifactStoreError::TotalSize)?;
        if length > MAX_ARTIFACT_BYTES {
            return Err(ArtifactStoreError::TotalSize);
        }
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count]).map_err(io_error)?;
    }
    output.flush().map_err(io_error)?;
    Ok((digest.finalize().into(), length))
}

fn hash_file(source: &Path) -> Result<([u8; 32], u64), ArtifactStoreError> {
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(o_nofollow())
        .open(source)
        .map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(count).map_err(|_| ArtifactStoreError::TotalSize)?)
            .ok_or(ArtifactStoreError::TotalSize)?;
        if length > MAX_ARTIFACT_BYTES {
            return Err(ArtifactStoreError::TotalSize);
        }
        digest.update(&buffer[..count]);
    }
    Ok((digest.finalize().into(), length))
}

#[cfg(target_os = "linux")]
const fn o_nofollow() -> i32 {
    0o400_000 | 0o2_000_000
}

#[cfg(not(target_os = "linux"))]
const fn o_nofollow() -> i32 {
    0
}

// `Result::map_err` supplies ownership; only the redacted kind is retained.
#[allow(clippy::needless_pass_by_value)]
fn io_error(error: std::io::Error) -> ArtifactStoreError {
    ArtifactStoreError::Io(error.kind())
}

/// Safe-import or immutable-store failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ArtifactStoreError {
    /// Canonical store root is unsafe.
    #[error("release artifact store root is invalid")]
    InvalidStoreRoot,
    /// Sealed build output is not an ordinary directory.
    #[error("sealed build output root is invalid")]
    InvalidSourceRoot,
    /// A path is not a normalized release-relative path.
    #[error("release artifact path is invalid")]
    InvalidPath,
    /// Tree includes a symlink or special filesystem object.
    #[error("sealed build output contains an unsafe object")]
    UnsafeObject,
    /// Hard-linked output could alias outside the sealed tree.
    #[error("sealed build output contains a hard link")]
    HardLink,
    /// Set-user-ID, set-group-ID, and sticky artifact modes are forbidden.
    #[error("sealed build output contains an unsupported mode")]
    UnsafeMode,
    /// Source metadata changed while copying.
    #[error("sealed build output changed during import")]
    SourceChanged,
    /// A stable opaque identity already names different canonical bytes.
    #[error("release artifact storage identity conflicts with canonical bytes")]
    ObjectConflict,
    /// Artifact count is empty or exceeds its bound.
    #[error("release artifact count is invalid")]
    FileCount,
    /// Aggregate bytes exceed the import ceiling.
    #[error("release artifact size is invalid")]
    TotalSize,
    /// Redacted I/O failure category.
    #[error("release artifact filesystem operation failed: {0:?}")]
    Io(std::io::ErrorKind),
}

#[cfg(test)]
mod tests {
    use super::{ArtifactStoreError, LocalArtifactStore, MAX_ARTIFACT_BYTES, MAX_ARTIFACT_FILES};
    use std::{
        fs::{self, hard_link},
        os::unix::{
            fs::{PermissionsExt, symlink},
            net::UnixListener,
        },
        process::Command,
    };
    use uuid::Uuid;

    fn fixture() -> (tempfile::TempDir, LocalArtifactStore) {
        let temporary = tempfile::tempdir().expect("temporary fixture");
        let store = temporary.path().join("store");
        fs::create_dir(&store).expect("store root");
        fs::set_permissions(&store, fs::Permissions::from_mode(0o700)).expect("store mode");
        (
            temporary,
            LocalArtifactStore::new(store).expect("valid store"),
        )
    }

    #[test]
    fn imports_deterministic_manifest_and_immutable_objects() {
        let (temporary, store) = fixture();
        let output = temporary.path().join("output");
        fs::create_dir_all(output.join("bin")).expect("output directories");
        fs::write(output.join("README"), b"documentation").expect("readme");
        fs::write(output.join("bin/reviewer"), b"exact executable").expect("executable");
        fs::set_permissions(
            output.join("bin/reviewer"),
            fs::Permissions::from_mode(0o755),
        )
        .expect("executable mode");

        let manifest = store.import(&output).expect("safe import");
        assert_eq!(
            manifest
                .iter()
                .map(|artifact| artifact.path.as_str())
                .collect::<Vec<_>>(),
            ["README", "bin/reviewer"]
        );
        assert_eq!(manifest[0].mode, 0o444);
        assert_eq!(manifest[1].mode, 0o555);
        for artifact in manifest {
            let path = store.resolve(artifact.storage_key).expect("stored object");
            assert_eq!(
                fs::metadata(path)
                    .expect("object metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o400
            );
        }
    }

    #[test]
    fn stable_import_reuses_verified_canonical_objects() {
        let (temporary, store) = fixture();
        let output = temporary.path().join("output");
        fs::create_dir(&output).expect("output");
        fs::write(output.join("agent"), b"stable bytes").expect("artifact");
        let operation_id = Uuid::new_v4();

        let first = store
            .import_for(operation_id, &output)
            .expect("first import");
        let second = store
            .import_for(operation_id, &output)
            .expect("retry import");

        assert_eq!(first, second);
    }

    #[test]
    fn rejects_symlinks_and_hardlinks() {
        let (temporary, store) = fixture();
        let output = temporary.path().join("output");
        fs::create_dir(&output).expect("output");
        symlink("/etc/passwd", output.join("escape")).expect("symlink");
        assert!(matches!(
            store.import(&output),
            Err(ArtifactStoreError::UnsafeObject)
        ));
        fs::remove_file(output.join("escape")).expect("remove symlink");
        fs::write(output.join("one"), b"aliased").expect("source");
        hard_link(output.join("one"), output.join("two")).expect("hard link");
        assert!(matches!(
            store.import(&output),
            Err(ArtifactStoreError::HardLink)
        ));
    }

    #[test]
    fn rejects_fifos_sockets_and_special_permission_modes() {
        let (temporary, store) = fixture();
        assert_eq!(
            store.import(std::path::Path::new("/dev/null")),
            Err(ArtifactStoreError::InvalidSourceRoot),
            "a character device cannot be used as an import root"
        );
        let output = temporary.path().join("output");
        fs::create_dir(&output).expect("output");

        let fifo = output.join("fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .expect("execute mkfifo")
                .success(),
            "create FIFO"
        );
        assert_eq!(store.import(&output), Err(ArtifactStoreError::UnsafeObject));
        fs::remove_file(&fifo).expect("remove FIFO");

        let socket_path = output.join("socket");
        let listener = UnixListener::bind(&socket_path).expect("create Unix socket");
        assert_eq!(store.import(&output), Err(ArtifactStoreError::UnsafeObject));
        drop(listener);
        fs::remove_file(&socket_path).expect("remove Unix socket");

        let privileged = output.join("setuid");
        fs::write(&privileged, b"ordinary bytes").expect("write setuid fixture");
        fs::set_permissions(&privileged, fs::Permissions::from_mode(0o4755))
            .expect("set special mode");
        assert_eq!(store.import(&output), Err(ArtifactStoreError::UnsafeMode));
    }

    #[test]
    fn rejects_the_first_values_over_file_and_byte_quotas() {
        let (temporary, store) = fixture();
        let output = temporary.path().join("output");
        fs::create_dir(&output).expect("output");
        for index in 0..=MAX_ARTIFACT_FILES {
            fs::File::create(output.join(format!("artifact-{index:04}")))
                .expect("create count-boundary artifact");
        }
        assert_eq!(store.import(&output), Err(ArtifactStoreError::FileCount));

        fs::remove_dir_all(&output).expect("remove count fixture");
        fs::create_dir(&output).expect("recreate output");
        let oversized = fs::File::create(output.join("oversized")).expect("oversized fixture");
        oversized
            .set_len(MAX_ARTIFACT_BYTES + 1)
            .expect("create sparse file over byte quota");
        assert_eq!(store.import(&output), Err(ArtifactStoreError::TotalSize));
    }
}

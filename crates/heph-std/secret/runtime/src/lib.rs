//! Ephemeral raw-secret filesystem construction and crash reconciliation.
//!
//! Values are written only beneath a configured memory-backed root. The
//! resulting directory is mounted read-only into a guest and may be destroyed
//! only after the caller records that the guest has been destroyed.

pub use heph_secret::{
    EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretDispatchInput,
    SecretMountManager, SecretMountMetadata, SecretMountProvider, SecretRuntimeError,
};
use runtime_types::RunId;
use secret_domain::{OpaqueRuntimeCredential, SecretSlotKey};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use vm_trait::VmMount;

/// Fixed guest path for raw secret files.
// Runtime control data occupies the read-only `/run/hephaestus` mount. Keep
// secrets as a sibling mount: libkrun cannot create a nested virtiofs target
// beneath that sealed control filesystem.
pub const GUEST_SECRET_PATH: &str = "/run/hephaestus-secrets";
/// Guest-visible file containing only the short-lived opaque broker/runtime
/// credential, never a secret value.
pub const RUNTIME_CREDENTIAL_FILE: &str = ".runtime-credential";
/// Maximum raw slots in one runtime.
pub const MAX_RAW_SECRET_FILES: usize = 32;
/// Maximum aggregate plaintext in one mount.
pub const MAX_RAW_SECRET_BYTES: usize = 256 * 1024;

/// Lifecycle of a raw secret mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretMountState {
    /// Files exist and may be attached to the exact guest.
    Materialized,
    /// The caller confirmed that the guest was destroyed.
    GuestDestroyed,
    /// Files and the per-run directory were removed.
    Destroyed,
}

/// Owned ephemeral mount handle.
#[derive(Debug)]
pub struct EphemeralSecretMount {
    run_id: RunId,
    host_path: PathBuf,
    slots: Vec<SecretSlotKey>,
    state: SecretMountState,
}

impl EphemeralSecretMount {
    /// Exact run bound to this directory.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Opaque host directory. Do not include it in logs or guest metadata.
    #[must_use]
    pub fn host_path(&self) -> &Path {
        &self.host_path
    }

    /// Non-secret symbolic slots for a separate runtime metadata document.
    #[must_use]
    pub fn slots(&self) -> &[SecretSlotKey] {
        &self.slots
    }

    /// Current cleanup lifecycle.
    #[must_use]
    pub const fn state(&self) -> SecretMountState {
        self.state
    }

    /// Builds the only VM mount contract permitted for this directory.
    #[must_use]
    pub fn vm_mount(&self) -> VmMount {
        VmMount {
            tag: format!("hs-{}", self.run_id.as_uuid().simple()),
            host_path: self.host_path.clone(),
            guest_path: PathBuf::from(GUEST_SECRET_PATH),
            read_only: true,
        }
    }

    /// Records that the VM and guest address space were destroyed.
    ///
    /// # Errors
    ///
    /// Returns an invalid-lifecycle error after cleanup.
    pub const fn mark_guest_destroyed(&mut self) -> Result<(), SecretRuntimeError> {
        match self.state {
            SecretMountState::Materialized | SecretMountState::GuestDestroyed => {
                self.state = SecretMountState::GuestDestroyed;
                Ok(())
            }
            SecretMountState::Destroyed => Err(SecretRuntimeError::InvalidLifecycle),
        }
    }

    /// Removes every file and the opaque directory after guest destruction.
    ///
    /// # Errors
    ///
    /// Fails closed if the caller has not confirmed guest destruction or if a
    /// path was replaced by a symlink/special file.
    pub fn destroy(&mut self) -> Result<(), SecretRuntimeError> {
        if self.state != SecretMountState::GuestDestroyed {
            return Err(SecretRuntimeError::GuestStillExists);
        }
        remove_secret_directory(&self.host_path)?;
        self.state = SecretMountState::Destroyed;
        Ok(())
    }
}

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

fn validate_root(config: &EphemeralSecretConfig) -> Result<(), SecretRuntimeError> {
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

fn is_memory_backed(path: &Path) -> Result<bool, SecretRuntimeError> {
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
            Some(credential) => materialize_with_authority(config, run_id, files, credential)?,
            None => materialize(config, run_id, files)?,
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

fn remove_secret_directory(path: &Path) -> Result<(), SecretRuntimeError> {
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
fn io_error(error: std::io::Error) -> SecretRuntimeError {
    SecretRuntimeError::Io(error.kind())
}

#[cfg(test)]
mod tests {
    use super::{
        EphemeralSecretConfig, GUEST_SECRET_PATH, RUNTIME_CREDENTIAL_FILE, RawSecretFile,
        SecretMountState, SecretRuntimeError, materialize, materialize_with_authority,
        reconcile_orphans,
    };
    use runtime_types::RunId;
    use secret_domain::{OpaqueRuntimeCredential, SecretSlotKey, SecretValue};
    use std::{
        collections::BTreeSet,
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    use uuid::Uuid;

    const SENTINEL: &[u8] = b"raw-mount-sentinel-71cf";

    fn config(temporary: &tempfile::TempDir) -> EphemeralSecretConfig {
        let root = temporary.path().join("secret-mounts");
        fs::create_dir(&root).expect("create root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("secure root");
        EphemeralSecretConfig {
            root,
            require_memory_filesystem: false,
        }
    }

    fn raw(slot: &str, value: &[u8]) -> RawSecretFile {
        RawSecretFile {
            slot: SecretSlotKey::parse(slot).expect("valid slot"),
            value: SecretValue::new(value).expect("valid value"),
        }
    }

    #[test]
    fn exact_read_only_contract_and_ordered_cleanup() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let config = config(&temporary);
        let credential = OpaqueRuntimeCredential::new([19_u8; 32]).expect("runtime credential");
        let mut mount = materialize_with_authority(
            &config,
            RunId::new(),
            vec![raw("model", SENTINEL)],
            &credential,
        )
        .expect("materialize");
        let file = mount.host_path().join("model");
        let authority = mount.host_path().join(RUNTIME_CREDENTIAL_FILE);
        assert_eq!(fs::read(&file).expect("read exact file"), SENTINEL);
        assert_eq!(
            fs::read(&authority).expect("read exact authority"),
            credential.expose()
        );
        assert_eq!(
            fs::metadata(&file)
                .expect("file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        assert_eq!(
            fs::metadata(&authority)
                .expect("authority metadata")
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        assert_eq!(
            fs::metadata(mount.host_path())
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o500
        );
        let vm_mount = mount.vm_mount();
        assert_eq!(vm_mount.guest_path.to_string_lossy(), GUEST_SECRET_PATH);
        assert!(vm_mount.read_only);
        assert!(matches!(
            mount.destroy(),
            Err(SecretRuntimeError::GuestStillExists)
        ));
        mount.mark_guest_destroyed().expect("guest destroyed");
        mount.destroy().expect("clean secret mount");
        assert_eq!(mount.state(), SecretMountState::Destroyed);
        assert!(
            !temporary
                .path()
                .to_string_lossy()
                .contains(std::str::from_utf8(SENTINEL).expect("sentinel UTF-8"))
        );
    }

    #[test]
    fn rejects_duplicates_bounds_and_unsafe_orphans() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let config = config(&temporary);
        let duplicated = materialize(
            &config,
            RunId::new(),
            vec![raw("model", b"a"), raw("model", b"b")],
        );
        assert!(matches!(duplicated, Err(SecretRuntimeError::DuplicateSlot)));

        let unsafe_path = config.root.join(Uuid::new_v4().simple().to_string());
        symlink(temporary.path(), &unsafe_path).expect("unsafe orphan symlink");
        assert!(matches!(
            reconcile_orphans(&config, &BTreeSet::new()),
            Err(SecretRuntimeError::UnsafeObject)
        ));
        fs::remove_file(unsafe_path).expect("remove test symlink");
    }

    #[test]
    fn reconciles_only_opaque_inactive_directories() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let config = config(&temporary);
        let live = materialize(&config, RunId::new(), vec![raw("live", b"a")])
            .expect("live materialization");
        let orphan = materialize(&config, RunId::new(), vec![raw("orphan", b"b")])
            .expect("orphan materialization");
        let live_name = live
            .host_path()
            .file_name()
            .expect("live name")
            .to_string_lossy()
            .into_owned();
        let live_set = BTreeSet::from([live_name]);
        assert_eq!(reconcile_orphans(&config, &live_set).expect("reconcile"), 1);
        assert!(live.host_path().exists());
        assert!(!orphan.host_path().exists());
    }
}

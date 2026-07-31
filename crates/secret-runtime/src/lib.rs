//! Ephemeral raw-secret filesystem construction and crash reconciliation.
//!
//! Values are written only beneath a configured memory-backed root. The
//! resulting directory is mounted read-only into a guest and may be destroyed
//! only after the caller records that the guest has been destroyed.

use async_trait::async_trait;
use forge_domain::{CommitSha, GitRef};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use run_domain::{Run, RunKind};
use run_orchestrator::{PreparedRunSecrets, RunSecretError, RunSecretManager};
use runtime_types::RunId;
use secret_application::{ResolveRunSecrets, SecretDispatchResolver, SecretRuntimeResolver};
use secret_domain::{
    DeliveryMode, ExecutionPhase, OpaqueRuntimeCredential, SecretCommandKey,
    SecretRuntimeSessionId, SecretSlotKey, SecretValue,
};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmMount;

/// Fixed guest path for raw secret files.
pub const GUEST_SECRET_PATH: &str = "/run/hephaestus/secrets";
/// Guest-visible file containing only the short-lived opaque broker/runtime
/// credential, never a secret value.
pub const RUNTIME_CREDENTIAL_FILE: &str = ".runtime-credential";
/// Maximum raw slots in one runtime.
pub const MAX_RAW_SECRET_FILES: usize = 32;
/// Maximum aggregate plaintext in one mount.
pub const MAX_RAW_SECRET_BYTES: usize = 256 * 1024;

/// Exact persisted dispatch provenance needed to resolve runtime secrets.
#[derive(Debug, Clone)]
pub struct SecretDispatchInput {
    /// Declared symbolic bindings from the immutable instance revision.
    pub secret_bindings: serde_json::Value,
    /// Authenticated actor selected by the dispatch request.
    pub actor_id: Option<Uuid>,
    /// Idempotent dispatch request identity.
    pub request_id: Option<Uuid>,
    /// Optional target ref for a normal run.
    pub git_ref: Option<String>,
    /// Optional target commit for a normal run.
    pub commit_sha: Option<String>,
}

/// Database boundary for ephemeral secret mount lifecycle and provenance.
///
/// The runtime owns encrypted-store, broker, and filesystem effects; this
/// narrow port owns only durable metadata and journal transitions. Adapters
/// must keep mount inserts and state updates transactional with the same
/// authorization queries used to resolve the exact run.
#[async_trait]
pub trait SecretMountMetadata: Send + Sync {
    /// Loads exact dispatch provenance for one run.
    async fn dispatch_input(
        &self,
        run: &Run,
    ) -> Result<Option<SecretDispatchInput>, RunSecretError>;
    /// Persists a materialized mount before guest attachment.
    async fn persist_mount(
        &self,
        run_id: RunId,
        mount: &EphemeralSecretMount,
    ) -> Result<(), RunSecretError>;
    /// Revalidates all active leases for one exact run.
    async fn authorized(&self, run: &Run) -> Result<bool, RunSecretError>;
    /// Returns a persisted materialized mount directory, if any.
    async fn materialized_directory(&self, run_id: RunId) -> Result<Option<Uuid>, RunSecretError>;
    /// Marks a mount destroyed after filesystem cleanup succeeds.
    async fn mark_destroyed(&self, run_id: RunId) -> Result<(), RunSecretError>;
    /// Returns opaque directories that belong to live runs.
    async fn live_directories(&self) -> Result<BTreeSet<String>, RunSecretError>;
    /// Marks cleaned-up run mounts destroyed in the durable journal.
    async fn mark_cleaned_mounts_destroyed(&self) -> Result<(), RunSecretError>;
}

/// Configuration for the local ephemeral mount boundary.
#[derive(Debug, Clone)]
pub struct EphemeralSecretConfig {
    /// Dedicated host root, normally below a `tmpfs` mounted at `/run`.
    pub root: PathBuf,
    /// Require the root to resolve to `tmpfs` or `ramfs`.
    pub require_memory_filesystem: bool,
}

/// One symbolic raw value to materialize.
pub struct RawSecretFile {
    /// Stable release slot and guest filename.
    pub slot: SecretSlotKey,
    /// Redacted plaintext wrapper.
    pub value: SecretValue,
}

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

/// Live-dispatch and ephemeral-mount integration for the run orchestrator.
pub struct SecretMountManager<M, D, R> {
    metadata: M,
    dispatch: D,
    runtime: R,
    config: EphemeralSecretConfig,
}

impl<M, D, R> SecretMountManager<M, D, R>
where
    M: SecretMountMetadata + 'static,
    D: SecretDispatchResolver,
    R: SecretRuntimeResolver,
{
    /// Validates configuration and creates a manager from separate command and
    /// narrow resolver services.
    ///
    /// # Errors
    ///
    /// Rejects an unsafe or non-memory-backed secret root.
    pub fn initialize(
        metadata: M,
        dispatch: D,
        runtime: R,
        config: EphemeralSecretConfig,
    ) -> Result<Self, RunSecretError> {
        validate_root(&config).map_err(secret_runtime_error)?;
        Ok(Self {
            metadata,
            dispatch,
            runtime,
            config,
        })
    }

    async fn dispatch_input(&self, run: &Run) -> Result<Option<DispatchInput>, RunSecretError> {
        let row = self
            .metadata
            .dispatch_input(run)
            .await?
            .ok_or_else(|| secret_error("exact secret dispatch provenance is unavailable"))?;
        let bindings: Vec<Uuid> =
            serde_json::from_value(row.secret_bindings.clone()).map_err(secret_serialization)?;
        if bindings.is_empty() {
            Ok(None)
        } else {
            Ok(Some(row))
        }
    }
}

#[async_trait]
impl<M, D, R> RunSecretManager for SecretMountManager<M, D, R>
where
    M: SecretMountMetadata + 'static,
    D: SecretDispatchResolver + 'static,
    R: SecretRuntimeResolver + 'static,
{
    async fn prepare(&self, run: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        let Some(input) = self.dispatch_input(run).await? else {
            return Ok(PreparedRunSecrets::default());
        };
        let actor_id = input
            .actor_id
            .ok_or_else(|| secret_error("secret-bearing run has no authenticated actor"))?;
        let request_id = input.request_id.unwrap_or_else(Uuid::new_v4);
        let identity = AuthenticatedIdentity::new(
            UserId::from_uuid(actor_id),
            "internal-run-dispatch",
            actor_id.to_string(),
            serde_json::json!({}),
            RequestId::from_uuid(request_id),
        );
        let target_ref = input
            .git_ref
            .map(GitRef::parse)
            .transpose()
            .map_err(|_| secret_error("secret-bearing run target ref is invalid"))?;
        let target_commit = input
            .commit_sha
            .map(CommitSha::parse)
            .transpose()
            .map_err(|_| secret_error("secret-bearing run target commit is invalid"))?;
        let command_key = SecretCommandKey::derive("dispatch", &[run.id.as_uuid().as_bytes()]);
        let authority = self
            .dispatch
            .resolve_for_dispatch(
                &identity,
                ResolveRunSecrets {
                    command_key,
                    session_id: SecretRuntimeSessionId::new(),
                    run_id: run.id,
                    instance_id: run.instance_id,
                    instance_revision_id: run.instance_revision_id,
                    attachment_id: run.attachment_id,
                    target_ref,
                    target_commit,
                    phase: match run.kind {
                        RunKind::Normal => ExecutionPhase::Normal,
                        RunKind::Update => ExecutionPhase::Update,
                    },
                    expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(10),
                },
            )
            .await
            .map_err(secret_service_error)?;
        let mut raw = Vec::new();
        for lease in &authority.leases {
            if lease.mode == DeliveryMode::Raw {
                let resolved = self
                    .runtime
                    .receive_raw(&authority.credential, run.id, lease.slot.clone())
                    .await
                    .map_err(secret_service_error)?;
                raw.push(RawSecretFile {
                    slot: resolved.slot,
                    value: resolved.value,
                });
            }
        }
        let mut mount =
            materialize_with_authority(&self.config, run.id, raw, &authority.credential)
                .map_err(secret_runtime_error)?;
        if let Err(error) = self.metadata.persist_mount(run.id, &mount).await {
            mount.mark_guest_destroyed().map_err(secret_runtime_error)?;
            mount.destroy().map_err(secret_runtime_error)?;
            return Err(error);
        }
        Ok(PreparedRunSecrets {
            mounts: vec![mount.vm_mount()],
        })
    }

    async fn reauthorize(&self, run: &Run) -> Result<(), RunSecretError> {
        if self.dispatch_input(run).await?.is_none() {
            return Ok(());
        }
        let authorized = self.metadata.authorized(run).await?;
        if authorized {
            Ok(())
        } else {
            Err(secret_error("live secret authority was revoked"))
        }
    }

    async fn destroy_after_guest(&self, run_id: RunId) -> Result<(), RunSecretError> {
        let directory = self.metadata.materialized_directory(run_id).await?;
        let Some(directory) = directory else {
            return Ok(());
        };
        destroy_confirmed(&self.config, directory).map_err(secret_runtime_error)?;
        self.metadata.mark_destroyed(run_id).await?;
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunSecretError> {
        let names = self.metadata.live_directories().await?;
        let removed = reconcile_orphans(&self.config, &names).map_err(secret_runtime_error)?;
        self.metadata.mark_cleaned_mounts_destroyed().await?;
        Ok(removed)
    }
}

type DispatchInput = SecretDispatchInput;

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

fn secret_error(message: impl Into<String>) -> RunSecretError {
    RunSecretError::redacted(message)
}

// Secret-service errors are already non-disclosing; the orchestration boundary
// nevertheless exposes only a stable failure class.
#[allow(clippy::needless_pass_by_value)]
fn secret_service_error(_error: secret_application::SecretServiceError) -> RunSecretError {
    secret_error("live secret dispatch failed")
}

#[allow(clippy::needless_pass_by_value)]
fn secret_runtime_error(_error: SecretRuntimeError) -> RunSecretError {
    secret_error("ephemeral secret filesystem operation failed")
}

#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::needless_pass_by_value)]
fn secret_serialization(_error: serde_json::Error) -> RunSecretError {
    secret_error("stored secret binding provenance is invalid")
}

/// Non-disclosing ephemeral runtime failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SecretRuntimeError {
    /// Root is missing, a symlink, or not a directory.
    #[error("ephemeral secret root is invalid")]
    InvalidRoot,
    /// Root is accessible by group or other users.
    #[error("ephemeral secret root permissions are unsafe")]
    InvalidRootPermissions,
    /// Production policy requires `tmpfs` or `ramfs`.
    #[error("ephemeral secret root is not memory-backed")]
    NotMemoryBacked,
    /// Slot count is empty or exceeds its ceiling.
    #[error("raw secret file count is invalid")]
    FileCount,
    /// Aggregate value bytes exceed their ceiling.
    #[error("raw secret aggregate size is invalid")]
    TotalSize,
    /// Two values selected one symbolic file.
    #[error("raw secret slot is duplicated")]
    DuplicateSlot,
    /// Guest destruction must precede filesystem destruction.
    #[error("raw secret guest still exists")]
    GuestStillExists,
    /// Cleanup lifecycle is invalid.
    #[error("raw secret mount lifecycle is invalid")]
    InvalidLifecycle,
    /// A cleanup path contains a symlink or special file.
    #[error("raw secret directory contains an unsafe object")]
    UnsafeObject,
    /// Orphan directory name is not an opaque UUID.
    #[error("ephemeral secret orphan is invalid")]
    InvalidOrphan,
    /// Redacted filesystem failure category.
    #[error("ephemeral secret filesystem operation failed: {0:?}")]
    Io(std::io::ErrorKind),
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

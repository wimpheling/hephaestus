use async_trait::async_trait;
use run_domain::Run;
use run_orchestrator::RunSecretError;
use runtime_types::RunId;
use secret_domain::{OpaqueRuntimeCredential, SecretSlotKey, SecretValue};
use std::{collections::BTreeSet, path::PathBuf};
use uuid::Uuid;
use vm_trait::VmMount;

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
#[async_trait]
pub trait SecretMountMetadata: Send + Sync {
    /// Loads exact dispatch provenance for one run.
    async fn dispatch_input(
        &self,
        run: &Run,
    ) -> Result<Option<SecretDispatchInput>, RunSecretError>;
    /// Persists the opaque directory before guest attachment.
    async fn persist_mount(
        &self,
        run_id: RunId,
        opaque_directory: Uuid,
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

/// Configuration for the ephemeral mount boundary.
#[derive(Debug, Clone)]
pub struct EphemeralSecretConfig {
    /// Dedicated host root, normally below a memory-backed filesystem.
    pub root: PathBuf,
    /// Require the root to resolve to tmpfs or ramfs.
    pub require_memory_filesystem: bool,
}

/// One symbolic raw value to materialize.
pub struct RawSecretFile {
    /// Stable release slot and guest filename.
    pub slot: SecretSlotKey,
    /// Redacted plaintext wrapper.
    pub value: SecretValue,
}

/// Provider result that crosses the core/standard boundary.
#[derive(Debug, Clone)]
pub struct MaterializedSecretMount {
    /// Opaque durable directory identity.
    pub opaque_directory: Uuid,
    /// Read-only guest mount contract.
    pub vm_mount: VmMount,
}

/// Filesystem or equivalent provider for ephemeral secret mounts.
pub trait SecretMountProvider: Send + Sync {
    /// Validates the configured mount root before manager startup.
    ///
    /// # Errors
    ///
    /// Returns a redacted lifecycle error when the root is unsafe.
    fn validate_config(&self, config: &EphemeralSecretConfig) -> Result<(), SecretRuntimeError>;
    /// Materializes exact raw values and optional runtime authority.
    ///
    /// # Errors
    ///
    /// Returns a redacted filesystem or bounds error when materialization fails.
    fn materialize(
        &self,
        config: &EphemeralSecretConfig,
        run_id: RunId,
        files: Vec<RawSecretFile>,
        credential: Option<&OpaqueRuntimeCredential>,
    ) -> Result<MaterializedSecretMount, SecretRuntimeError>;
    /// Removes a mount when durable persistence failed before guest attach.
    ///
    /// # Errors
    ///
    /// Returns a redacted cleanup error when removal fails.
    fn discard_materialized(
        &self,
        config: &EphemeralSecretConfig,
        opaque_directory: Uuid,
    ) -> Result<(), SecretRuntimeError>;
    /// Removes one persisted mount after guest destruction.
    ///
    /// # Errors
    ///
    /// Returns a redacted cleanup error when removal fails.
    fn destroy_confirmed(
        &self,
        config: &EphemeralSecretConfig,
        opaque_directory: Uuid,
    ) -> Result<(), SecretRuntimeError>;
    /// Removes opaque directories that are not live.
    ///
    /// # Errors
    ///
    /// Returns a redacted cleanup error when reconciliation fails.
    fn reconcile_orphans(
        &self,
        config: &EphemeralSecretConfig,
        live_directories: &BTreeSet<String>,
    ) -> Result<usize, SecretRuntimeError>;
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
    /// Production policy requires tmpfs or ramfs.
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
    use super::{MaterializedSecretMount, SecretMountProvider};
    use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

    struct TestProvider;

    impl SecretMountProvider for TestProvider {
        fn validate_config(
            &self,
            _config: &super::EphemeralSecretConfig,
        ) -> Result<(), super::SecretRuntimeError> {
            Ok(())
        }

        fn materialize(
            &self,
            _config: &super::EphemeralSecretConfig,
            _run_id: super::RunId,
            _files: Vec<super::RawSecretFile>,
            _credential: Option<&secret_domain::OpaqueRuntimeCredential>,
        ) -> Result<MaterializedSecretMount, super::SecretRuntimeError> {
            Err(super::SecretRuntimeError::InvalidRoot)
        }

        fn discard_materialized(
            &self,
            _config: &super::EphemeralSecretConfig,
            _opaque_directory: uuid::Uuid,
        ) -> Result<(), super::SecretRuntimeError> {
            Ok(())
        }

        fn destroy_confirmed(
            &self,
            _config: &super::EphemeralSecretConfig,
            _opaque_directory: uuid::Uuid,
        ) -> Result<(), super::SecretRuntimeError> {
            Ok(())
        }

        fn reconcile_orphans(
            &self,
            _config: &super::EphemeralSecretConfig,
            _live_directories: &BTreeSet<String>,
        ) -> Result<usize, super::SecretRuntimeError> {
            Ok(0)
        }
    }

    #[test]
    fn provider_port_is_object_safe_and_neutral() {
        let provider: Arc<dyn SecretMountProvider> = Arc::new(TestProvider);
        let config = super::EphemeralSecretConfig {
            root: PathBuf::from("/unused"),
            require_memory_filesystem: false,
        };
        assert!(provider.validate_config(&config).is_ok());
    }
}

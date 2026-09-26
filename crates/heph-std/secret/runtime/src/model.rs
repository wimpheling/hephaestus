//! Ephemeral mount model and lifecycle operations.

use runtime_types::RunId;
use secret_domain::SecretSlotKey;
use std::path::{Path, PathBuf};
use vm_trait::VmMount;

use super::SecretRuntimeError;

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
    pub(super) run_id: RunId,
    pub(super) host_path: PathBuf,
    pub(super) slots: Vec<SecretSlotKey>,
    pub(super) state: SecretMountState,
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
        super::filesystem::remove_secret_directory(&self.host_path)?;
        self.state = SecretMountState::Destroyed;
        Ok(())
    }
}

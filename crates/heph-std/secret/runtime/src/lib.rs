//! Ephemeral raw-secret filesystem construction and crash reconciliation.
//!
//! Values are written only beneath a configured memory-backed root. The
//! resulting directory is mounted read-only into a guest and may be destroyed
//! only after the caller records that the guest has been destroyed.

pub use heph_secret::{
    EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretDispatchInput,
    SecretMountManager, SecretMountMetadata, SecretMountProvider, SecretRuntimeError,
};

mod filesystem;
mod materialize;
mod model;

pub use filesystem::{FilesystemSecretMountProvider, destroy_confirmed, reconcile_orphans};
pub use materialize::{materialize, materialize_with_authority};
pub use model::{
    EphemeralSecretMount, GUEST_SECRET_PATH, MAX_RAW_SECRET_BYTES, MAX_RAW_SECRET_FILES,
    RUNTIME_CREDENTIAL_FILE, SecretMountState,
};

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;

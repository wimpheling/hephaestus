//! Provider-neutral secret runtime composition and lifecycle ports.
//!
//! The facade exposes the lifecycle contracts and [`SecretMountManager`].
//! Database adapters, provider-owned mount handles, and plaintext value types
//! remain in their owning crates.
//!
//! ```compile_fail
//! use heph_secret::PostgresSecretMountMetadata;
//! ```
//!
//! ```compile_fail
//! use heph_secret::{EphemeralSecretMount, FilesystemSecretMountProvider};
//! ```
//!
//! ```compile_fail
//! use heph_secret::SecretValue;
//!
//! fn read_plaintext(value: SecretValue) -> Vec<u8> {
//!     value.expose().to_vec()
//! }
//! ```

mod contracts;
mod manager;

pub use contracts::{
    EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretDispatchInput,
    SecretMountMetadata, SecretMountProvider, SecretRuntimeError,
};
pub use manager::SecretMountManager;

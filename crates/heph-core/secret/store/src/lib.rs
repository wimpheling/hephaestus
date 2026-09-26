//! Authenticated envelope encryption and the provider-neutral secret storage
//! boundary.
//!
//! `PostgreSQL` stores [`EncryptedSecretVersion`] metadata and ciphertext. The
//! versioned host key provider remains outside `PostgreSQL`. A unique random
//! data key and unique nonces isolate every immutable secret version.

mod error;
mod keys;
mod model;
mod store;

pub use error::SecretStoreError;
pub use keys::{KeyProvider, LocalKeyProvider};
pub use model::{ALGORITHM, EncryptedSecretVersion, VersionContext};
pub use store::EncryptedStore;

#[cfg(test)]
#[path = "tests/secret_store.rs"]
mod tests;

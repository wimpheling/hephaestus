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
pub use keys::KeyProvider;
pub use model::{ALGORITHM, EncryptedSecretVersion, VersionContext};
pub use store::EncryptedStore;

#[cfg(any(test, feature = "test-fixtures"))]
pub use test_keys::TestKeyProvider;

#[cfg(any(test, feature = "test-fixtures"))]
mod test_keys;

#[cfg(test)]
#[path = "tests/secret_store.rs"]
mod tests;

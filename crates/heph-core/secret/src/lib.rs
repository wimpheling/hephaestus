//! Provider-neutral secret runtime composition and lifecycle ports.

mod contracts;
mod manager;

pub use contracts::{
    EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretDispatchInput,
    SecretMountMetadata, SecretMountProvider, SecretRuntimeError,
};
pub use manager::SecretMountManager;

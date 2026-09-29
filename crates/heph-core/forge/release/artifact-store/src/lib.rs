//! Safe one-way import from an untrusted sealed build-output directory.
//!
//! Import accepts only ordinary directories and single-linked regular files,
//! hashes bytes while copying into an opaque immutable store, and returns a
//! deterministic path-sorted manifest. Repository-controlled paths never
//! select canonical host storage locations.

#[path = "artifact_store/errors.rs"]
mod errors;
#[path = "artifact_store/hashing.rs"]
mod hashing;
#[path = "artifact_store/store.rs"]
mod store;
#[path = "artifact_store/validation.rs"]
mod validation;

pub use errors::ArtifactStoreError;
pub use store::{ImportedArtifact, LocalArtifactStore};
/// Maximum files in one release import.
pub const MAX_ARTIFACT_FILES: usize = 4_096;
/// Maximum aggregate imported bytes.
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

#[cfg(test)]
#[path = "artifact_store/tests.rs"]
mod tests;

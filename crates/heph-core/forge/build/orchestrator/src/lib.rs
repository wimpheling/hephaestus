//! Isolated build execution from an exact Git commit into the one-way
//! immutable release-artifact importer.
//!
//! The build guest receives a read-only source tree and one empty writable
//! output tree. It receives no canonical Git directory, release-store path,
//! instance state volume, secret mount, or host credential.

mod repository;
pub use repository::{
    BuildInput, BuildRepository, BuildRepositoryError, ClaimedBuild, FinalizationBuild,
    RecoverableBuild,
};

mod executor;
pub use executor::{BuildExecutionError, BuildExecutionResult, BuildExecutor, BuildExecutorConfig};

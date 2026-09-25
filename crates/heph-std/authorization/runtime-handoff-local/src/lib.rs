//! Trusted-host adapters for temporary runtime credential handoff.

mod runtime;
mod runtime_git;

pub use runtime::EncryptedFileHandoffStore;
pub use runtime_git::EncryptedFileRuntimeGitHandoffStore;

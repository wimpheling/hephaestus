//! Trusted-host adapters for temporary runtime credential handoff.

mod envelope;
mod pinned;
mod runtime;
mod runtime_git;

pub use runtime::EncryptedFileHandoffStore;
pub use runtime_git::EncryptedFileRuntimeGitHandoffStore;

#[cfg(test)]
mod expired_test_support;

//! Strict, provider-neutral Git capability grammar.
//!
//! The types in this crate describe authority; they do not authenticate a
//! credential or inspect a repository. Transport adapters must obtain trusted
//! repository state and use it to construct the transitions checked here.

mod ceiling;
mod errors;
mod git_hash;
mod glob_validation;
mod globs;
mod identifiers;
mod limits;
mod policies;
mod scope;
mod transitions;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Current version of the normalized Git capability grammar.
pub const GRAMMAR_VERSION: u16 = 1;

const MAX_REF_GLOB_BYTES: usize = 512;
const MAX_PATH_GLOB_BYTES: usize = 1_024;
const MAX_GLOBS: usize = 256;
const MAX_REQUEST_BYTES: u64 = 16 * 1_024 * 1_024;
const MAX_PACK_BYTES: u64 = 1_024 * 1_024 * 1_024;
const MAX_OBJECTS: u32 = 1_000_000;
const MAX_REF_UPDATES: u16 = 256;

pub use ceiling::{BoundGitCapability, GitCapabilityCeiling, GitCapabilityCeilingInput};
pub use errors::GitCapabilityError;
pub use git_hash::GitCapabilityHash;
pub use globs::{ChangedPathGlob, RefGlob};
pub use identifiers::{GitOperation, RepositoryId};
pub use limits::TransferLimits;
pub use policies::{
    BranchRefPolicy, BranchUpdatePolicy, RefMutationPermission, RefNamespacePolicy, RefUpdatePolicy,
};
pub use scope::{GitCapabilityScope, GitCapabilityScopeInput};
pub use transitions::{PathChange, ReceiveUpdate, RefTransition};

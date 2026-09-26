//! Local exact-commit workspaces and trusted Git result publication.

use std::sync::Arc;
use workspace_domain::{ResultRepository, WorkspaceMetadataRepository};

mod api;
mod artifacts;
mod common;
mod config;
mod errors;
mod filesystem;
mod finalize;
mod git;
mod git_tree;
mod import;
mod import_tree;
mod manager;
mod materialize;
mod recovery;
mod runtime_git;
mod runtime_materialize;
mod safety;
mod types;
mod validation;
mod workspace;

pub use config::{LocalWorkspaceConfig, WorkspaceLimits};
pub use errors::LocalWorkspaceError;

/// Filesystem/Git workspace and result implementation over provider-neutral ports.
#[derive(Clone)]
pub struct LocalWorkspaceManager {
    pub(crate) metadata: Arc<dyn WorkspaceMetadataRepository>,
    pub(crate) results: Arc<dyn ResultRepository>,
    pub(crate) config: LocalWorkspaceConfig,
}

#[cfg(test)]
mod tests;

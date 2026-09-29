use crate::{BuildInput, BuildRepository, ClaimedBuild, FinalizationBuild};
use agent_config::{BuildArtifactKind, BuildConfig, NetworkProfile};
use release_artifact_store::{ImportedArtifact, LocalArtifactStore};
use release_domain::{
    ArtifactKind, BuildRequestId, ReleaseAgentId, ReleaseArtifactId, ReleaseCommandKey, ReleaseId,
    ReleaseVersion,
};
use release_postgres::ReleaseService;
use release_service::{CompleteBuild, ReleaseArtifactInput};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::broadcast;
use uuid::Uuid;
use vm_trait::{
    GuestCommand, LogStream, NetworkMode, RootFilesystem, StopMode, VmEvent, VmId, VmMount,
    VmProvider, VmResources, VmSpec,
};

const SOURCE_GUEST_PATH: &str = "/workspace/source";
const OUTPUT_GUEST_PATH: &str = "/workspace/output";
const BUILD_GUEST_ENV: &str = "HEPH_BUILD_GUEST";
const MAX_SOURCE_ENTRIES: usize = 100_000;
const MAX_SOURCE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_BUILD_LOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_BUILD_METRICS: usize = 4_096;

/// Host paths, root images, and execution bounds for isolated builds.
#[derive(Debug, Clone)]
pub struct BuildExecutorConfig {
    /// Private transient build workspace root.
    pub workspace_root: PathBuf,
    /// Bare canonical repository root. Only the host materializer reads it.
    pub repository_root: PathBuf,
    /// Absolute trusted Git executable.
    pub git_binary: PathBuf,
    /// Materialized immutable OCI images, keyed by exact digest-pinned
    /// reference. The daemon refreshes this cache only after its trusted OCI
    /// materializer has atomically installed a rootfs.
    pub image_filesystems: Arc<RwLock<BTreeMap<String, RootFilesystem>>>,
    /// Maximum wall-clock duration for one build guest.
    pub timeout: Duration,
}

/// Stable result of one imported draft-producing build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildExecutionResult {
    /// Exact durable build request.
    pub build_request_id: BuildRequestId,
    /// Stable draft release created from the imported manifest.
    pub release_id: ReleaseId,
    /// Exported release agent.
    pub release_agent_id: ReleaseAgentId,
    /// Deterministic build-derived draft version.
    pub release_version: ReleaseVersion,
    /// Number of safely imported regular artifacts.
    pub artifact_count: usize,
}

/// PostgreSQL-coordinated isolated build worker.
pub struct BuildExecutor {
    repository: Arc<dyn BuildRepository>,
    provider: Arc<dyn VmProvider>,
    artifacts: LocalArtifactStore,
    releases: Arc<ReleaseService>,
    config: BuildExecutorConfig,
}

mod cleanup;
mod errors;
mod monitor;
mod output;
mod run;
mod state;
mod verify;
mod workspace;

use cleanup::{cleanup_workspace, filesystem, validate_private_directory};
pub use errors::BuildExecutionError;
use monitor::collect_execution;
use output::{
    artifact_manifest, release_inputs, seal_output, stored_release_inputs,
    validate_declared_outputs,
};
use workspace::{PreparedBuildWorkspace, prepare_workspace};

#[cfg(test)]
mod tests;

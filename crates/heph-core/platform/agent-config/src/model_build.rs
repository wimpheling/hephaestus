//! Build, publication, workspace, and network declarations.
use capability_domain::CapabilitySlotKey;
use serde::{Deserialize, Serialize};

const fn default_true() -> bool {
    true
}
/// One declared build output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildArtifact {
    /// Normalized path relative to the writable build workspace.
    pub path: String,
    /// Expected output kind.
    pub kind: BuildArtifactKind,
    /// Optional bounded media type.
    #[serde(default)]
    pub media_type: Option<String>,
}

/// Safe importer output kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildArtifactKind {
    /// One regular file.
    File,
    /// A traversal-free tree of regular files and directories.
    Directory,
    /// One executable regular file.
    Executable,
}

/// Compute resource limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Virtual CPU count.
    pub vcpus: u8,
    /// Guest memory in mebibytes.
    pub memory_mib: u32,
}

/// Repository workspace mount intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMount {
    /// Whether the exact received tree is mounted in the guest.
    pub mount: bool,
    /// Guest mount path.
    pub path: String,
    /// Whether the mount must be read-only.
    #[serde(default = "default_true")]
    pub read_only: bool,
}

/// Release-owned repository publication contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationConfig {
    /// Exact publication path selected by this immutable release.
    #[serde(default)]
    pub mode: PublicationMode,
    /// Repository capability slot used for a runtime Git worktree and remote.
    ///
    /// This is a symbolic release-owned name, never an attachment or tenant
    /// repository identifier.
    #[serde(default)]
    pub repository_slot: Option<CapabilitySlotKey>,
}

impl Default for PublicationConfig {
    fn default() -> Self {
        Self {
            mode: PublicationMode::Proposal,
            repository_slot: None,
        }
    }
}

/// Immutable release publication modes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationMode {
    /// A trusted host imports a detached writable tree to a controlled result
    /// ref; the guest receives no Git metadata or write remote.
    #[default]
    Proposal,
    /// A future capability-scoped runtime worktree may publish through normal
    /// Git receive. Declaring the mode does not itself grant repository access.
    RuntimeGit,
}

impl PublicationMode {
    /// Returns the stable database and wire representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposal => "proposal",
            Self::RuntimeGit => "runtime_git",
        }
    }

    /// Whether this mode may be considered for a capability-scoped Git write
    /// remote once the runtime Git transport is implemented.
    #[must_use]
    pub const fn permits_git_write_remote(self) -> bool {
        matches!(self, Self::RuntimeGit)
    }
}

/// Persistent state-volume selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateVolume {
    /// Whether the agent receives its persistent state volume.
    pub enabled: bool,
}

/// Durable result-artifact requests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultConfig {
    /// Relative regular-file paths to retain in addition to the full result
    /// manifest and patch.
    #[serde(default)]
    pub declared_files: Vec<String>,
}

/// Guest network selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Named provider-neutral network profile.
    pub profile: NetworkProfile,
}

/// Provider-neutral network profiles supported in the initial schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProfile {
    /// No guest network.
    Disabled,
    /// Egress-capable user-mode network.
    Egress,
    /// Only host broker connectivity.
    BrokerOnly,
}

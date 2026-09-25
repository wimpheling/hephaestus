//! Core agent and image configuration declarations.
use super::{
    model_build::{
        BuildArtifact, NetworkConfig, PublicationConfig, ResourceLimits, ResultConfig, StateVolume,
        WorkspaceMount,
    },
    model_capabilities::CapabilitySlotDeclaration,
    model_parameters::{ParameterDeclaration, SecretSlotDeclaration},
    model_runtime::{TriggerConfig, UpdateHookConfig},
};
use serde::{Deserialize, Serialize};

/// A successfully parsed and validated configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// Configuration schema version.
    pub version: u32,
    /// Agent identity.
    pub agent: AgentIdentity,
    /// Isolated build definition, required by version 2.
    #[serde(default)]
    pub build: Option<BuildConfig>,
    /// Guest process and its selected OCI image.
    pub guest: GuestConfig,
    /// Compute limits.
    pub resources: ResourceLimits,
    /// Repository workspace mount intent.
    pub workspace: WorkspaceMount,
    /// How successful runtime repository changes may be published.
    ///
    /// Omitted declarations resolve to proposal mode so configurations that
    /// predate publication modes retain their controlled-import behavior.
    #[serde(default)]
    pub publication: PublicationConfig,
    /// Persistent agent-state volume intent.
    pub state_volume: StateVolume,
    /// Files copied into durable result artifacts after a successful run.
    #[serde(default)]
    pub results: ResultConfig,
    /// Guest networking profile.
    pub network: NetworkConfig,
    /// Run trigger policy.
    pub triggers: TriggerConfig,
    /// Typed project-selectable parameters.
    #[serde(default)]
    pub parameters: Vec<ParameterDeclaration>,
    /// Symbolic secret/credential capability slots. These declarations never
    /// name tenant secrets.
    #[serde(default)]
    pub secret_slots: Vec<SecretSlotDeclaration>,
    /// Symbolic requirements for exact Hephaestus resources. Release source
    /// declares only a ceiling; an instance revision supplies exact bindings.
    #[serde(default)]
    pub capability_slots: Vec<CapabilitySlotDeclaration>,
    /// Optional isolated candidate-release update hook.
    #[serde(default)]
    pub update_hook: Option<UpdateHookConfig>,
}

/// Stable exported identity and human-readable display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentity {
    /// Mutable display name.
    pub name: String,
    /// Stable repository-scoped exported key, required by version 2.
    #[serde(default)]
    pub key: Option<String>,
}

/// Guest process definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestConfig {
    /// Catalog key for the OCI image that executes this guest.
    pub image: ImageSelection,
    /// Path relative to the immutable `/release` mount.
    pub command: String,
    /// Arguments excluding the executable.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Release-relative working directory.
    pub working_directory: String,
}

/// Isolated build guest contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    /// Catalog key for the OCI image that executes this build.
    pub image: ImageSelection,
    /// Absolute executable inside the pinned build root image.
    pub command: String,
    /// Fixed arguments.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Absolute directory inside the build guest.
    pub working_directory: String,
    /// Build-specific resource limits.
    pub resources: ResourceLimits,
    /// Build network ceiling.
    pub network: NetworkConfig,
    /// Declared regular-file or directory outputs.
    pub artifacts: Vec<BuildArtifact>,
    /// Refs whose accepted updates request builds.
    #[serde(default)]
    pub triggers: Vec<String>,
}

/// A declarative OCI image identity resolved when a build is requested.
///
/// `key` selects a reviewed platform execution image. `project_image` selects
/// a ready, materialized image owned by the project containing the agent. The
/// latter is deliberately accepted only for build contracts: a repository
/// image is not yet a runtime guest/deployment image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSelection {
    /// Stable key in the reviewed OCI image catalog.
    #[serde(default)]
    pub key: Option<String>,
    /// Stable key of a ready project-owned OCI image.
    #[serde(default)]
    pub project_image: Option<String>,
}

/// Repository-owned OCI image definitions read from `heph.images.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryOciImagesConfig {
    /// Manifest schema version.
    pub version: u32,
    /// Builder definitions owned by this repository.
    #[serde(default)]
    pub images: Vec<RepositoryOciImageConfig>,
}

/// One repository-local OCI image definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryOciImageConfig {
    /// Stable repository-local key.
    pub key: String,
    /// Human-readable non-secret display name.
    pub display_name: String,
    /// Isolated OCI build input.
    pub build: RepositoryOciImageBuildConfig,
}

/// OCI inputs accepted from a repository manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryOciImageBuildConfig {
    /// Repository-relative Dockerfile path.
    pub dockerfile: String,
    /// Repository-relative build-context path.
    pub context: String,
    /// Platform image selected as the immutable base during production.
    pub base: ImageSelection,
}

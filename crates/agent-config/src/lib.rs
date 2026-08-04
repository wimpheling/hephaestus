//! Versioned `agent.toml` parsing, validation, and trigger matching.

use forge_domain::GitRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fmt::Write as _, path::Path};

/// Reusable release and typed-instance configuration version.
pub const REUSABLE_RELEASE_VERSION: u32 = 2;

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
    /// Guest process.
    pub guest: GuestConfig,
    /// Compute limits.
    pub resources: ResourceLimits,
    /// Immutable guest root image.
    pub root_image: RootImage,
    /// Repository workspace mount intent.
    pub workspace: WorkspaceMount,
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
    /// Absolute executable inside the pinned build root image.
    pub command: String,
    /// Fixed arguments.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Absolute directory inside the build guest.
    pub working_directory: String,
    /// Legacy digest-pinned build root image. New configurations should use
    /// [`BuilderSelection`] so the platform resolves the digest from its
    /// approved catalog at build-request time.
    #[serde(default)]
    pub root_image: Option<String>,
    /// Catalog identity for the build root image.
    #[serde(default)]
    pub builder: Option<BuilderSelection>,
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

/// A declarative builder identity resolved by the owning project and platform
/// catalog before a VM is started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BuilderSelection {
    /// A platform-curated builder key.
    Platform {
        /// Stable platform catalog key.
        key: String,
    },
    /// A project-owned prepared builder identity.
    Project {
        /// Opaque UUID of the project-owned builder definition.
        id: String,
    },
    /// A repository-owned builder declared by the exact source commit.
    Repository {
        /// Stable key from the repository's `heph.builders.toml`.
        key: String,
    },
}

/// Schema version for a repository's OCI builder manifest.
pub const REPOSITORY_BUILDERS_VERSION: u32 = 1;

/// Repository-owned OCI builder definitions read from `heph.builders.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryBuildersConfig {
    /// Manifest schema version.
    pub version: u32,
    /// Builder definitions owned by this repository.
    #[serde(default)]
    pub builders: Vec<RepositoryBuilderConfig>,
}

/// One repository-local OCI builder definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryBuilderConfig {
    /// Stable repository-local key.
    pub key: String,
    /// Human-readable non-secret display name.
    pub display_name: String,
    /// Isolated OCI build input.
    pub oci: RepositoryBuilderOciConfig,
}

/// OCI inputs accepted from a repository manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryBuilderOciConfig {
    /// Repository-relative Dockerfile path.
    pub dockerfile: String,
    /// Repository-relative build-context path.
    pub context: String,
    /// Approved platform catalog key, never an OCI reference.
    pub base: String,
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

/// Root image selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootImage {
    /// Immutable image reference, normally digest-pinned.
    pub reference: String,
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

/// Typed parameter declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterDeclaration {
    /// Stable bounded name.
    pub name: String,
    /// Type and explicit bounds.
    #[serde(flatten)]
    pub value_type: ParameterType,
    /// Whether a value or default is required.
    #[serde(default)]
    pub required: bool,
    /// Optional TOML default.
    #[serde(default)]
    pub default: Option<ParameterDefault>,
    /// Whether UI and telemetry redact the ordinary value.
    #[serde(default)]
    pub sensitive: bool,
}

/// Bounded parameter type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParameterType {
    /// UTF-8 string bounds.
    String {
        /// Inclusive minimum characters.
        minimum_length: u16,
        /// Inclusive maximum characters.
        maximum_length: u16,
    },
    /// Signed integer bounds.
    Integer {
        /// Inclusive minimum.
        minimum: i64,
        /// Inclusive maximum.
        maximum: i64,
    },
    /// Boolean.
    Boolean,
    /// Exact bounded choices.
    Enum {
        /// Allowed strings.
        values: Vec<String>,
    },
}

/// Supported TOML default values without coercion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterDefault {
    /// String or enum default.
    String(String),
    /// Integer default.
    Integer(i64),
    /// Boolean default.
    Boolean(bool),
}

/// Symbolic secret declaration without tenant identifiers or values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretSlotDeclaration {
    /// Stable symbolic key.
    pub key: String,
    /// Human-readable non-secret purpose.
    pub purpose: String,
    /// Whether a runnable instance revision requires a binding.
    #[serde(default)]
    pub required: bool,
    /// Accepted delivery modes.
    pub delivery_modes: Vec<SecretDeliveryMode>,
    /// Accepted execution phases.
    pub phases: Vec<SecretPhase>,
    /// Optional exact broker destination ceiling.
    #[serde(default)]
    pub destinations: Vec<String>,
}

/// Declared secret delivery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretDeliveryMode {
    /// Ephemeral raw-file delivery.
    Raw,
    /// Non-disclosing broker use.
    Brokered,
}

/// Declared secret execution phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretPhase {
    /// Ordinary run.
    Normal,
    /// Candidate update hook.
    Update,
}

/// Candidate-release update hook executed only inside an isolated guest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateHookConfig {
    /// Executable relative to `/release`.
    pub command: String,
    /// Fixed arguments, without a host shell.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Wall-clock timeout.
    pub timeout_seconds: u32,
    /// Hook-specific resources within the release ceiling.
    pub resources: ResourceLimits,
}

/// Push-trigger selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerConfig {
    /// Whether accepted pushes may create runs.
    pub push: bool,
    /// Fully-qualified refs or terminal `/*` prefixes that may trigger.
    #[serde(default)]
    pub refs: Vec<String>,
}

impl TriggerConfig {
    /// Returns whether an accepted update to `git_ref` triggers a run.
    #[must_use]
    pub fn matches(&self, git_ref: &GitRef) -> bool {
        self.push
            && (self.refs.is_empty()
                || self.refs.iter().any(|pattern| {
                    pattern.strip_suffix("/*").map_or_else(
                        || pattern == git_ref.as_str(),
                        |prefix| {
                            git_ref
                                .as_str()
                                .strip_prefix(prefix)
                                .is_some_and(|suffix| suffix.starts_with('/'))
                        },
                    )
                }))
    }
}

/// Stable hash of the original configuration bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConfigHash(String);

impl ConfigHash {
    /// Returns the lowercase SHA-256 digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One structured configuration diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable diagnostic code.
    pub code: String,
    /// Field path, when the error belongs to one field.
    pub path: Option<String>,
    /// Human-readable explanation.
    pub message: String,
}

/// Result of parsing and validating one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConfig {
    /// Hash of the exact source bytes.
    pub hash: ConfigHash,
    /// Hash of deterministic normalized configuration serialization.
    pub normalized_hash: Option<ConfigHash>,
    /// Validated configuration, absent on failure.
    pub config: Option<AgentConfig>,
    /// Structured parser and validation diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of parsing one repository `heph.builders.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRepositoryBuilders {
    /// Hash of the exact source bytes.
    pub hash: ConfigHash,
    /// Validated manifest, absent on failure.
    pub config: Option<RepositoryBuildersConfig>,
    /// Structured parser and validation diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses and validates `agent.toml`.
#[must_use]
pub fn parse(source: &[u8]) -> ParsedConfig {
    let source_hash = hash(source);
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            return ParsedConfig {
                hash: source_hash,
                normalized_hash: None,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_utf8"),
                    path: None,
                    message: error.to_string(),
                }],
            };
        }
    };
    let config = match toml::from_str::<AgentConfig>(text) {
        Ok(config) => config,
        Err(error) => {
            return ParsedConfig {
                hash: source_hash,
                normalized_hash: None,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_toml"),
                    path: error
                        .span()
                        .map(|span| format!("bytes {}..{}", span.start, span.end)),
                    message: error.message().to_owned(),
                }],
            };
        }
    };
    let mut diagnostics = validate(&config);
    let normalized_hash = if diagnostics.is_empty() {
        toml::to_string(&config).map_or_else(
            |_| {
                diagnostics.push(Diagnostic {
                    code: String::from("normalization_failed"),
                    path: None,
                    message: String::from("validated configuration could not be normalized"),
                });
                None
            },
            |normalized| Some(hash(normalized.as_bytes())),
        )
    } else {
        None
    };
    ParsedConfig {
        hash: source_hash,
        normalized_hash,
        config: diagnostics.is_empty().then_some(config),
        diagnostics,
    }
}

/// Parses and validates a repository's root-level `heph.builders.toml`.
///
/// This function only validates the declarative source contract. The receive
/// workflow verifies the declared paths against the exact Git tree and resolves
/// `oci.base` to an approved digest-pinned platform image transactionally.
#[must_use]
pub fn parse_repository_builders(source: &[u8]) -> ParsedRepositoryBuilders {
    let source_hash = hash(source);
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            return ParsedRepositoryBuilders {
                hash: source_hash,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_utf8"),
                    path: None,
                    message: error.to_string(),
                }],
            };
        }
    };
    let config = match toml::from_str::<RepositoryBuildersConfig>(text) {
        Ok(config) => config,
        Err(error) => {
            return ParsedRepositoryBuilders {
                hash: source_hash,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_toml"),
                    path: error
                        .span()
                        .map(|span| format!("bytes {}..{}", span.start, span.end)),
                    message: error.message().to_owned(),
                }],
            };
        }
    };
    let diagnostics = validate_repository_builders(&config);
    ParsedRepositoryBuilders {
        hash: source_hash,
        config: diagnostics.is_empty().then_some(config),
        diagnostics,
    }
}

fn hash(source: &[u8]) -> ConfigHash {
    let digest = Sha256::digest(source);
    let mut value = String::with_capacity(64);
    for byte in digest {
        write!(&mut value, "{byte:02x}").expect("writing to a String cannot fail");
    }
    ConfigHash(value)
}

fn validate_repository_builders(config: &RepositoryBuildersConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != REPOSITORY_BUILDERS_VERSION {
        diagnostic(
            &mut diagnostics,
            "unsupported_repository_builders_version",
            "version",
            format!(
                "repository builder version {} is unsupported; expected {REPOSITORY_BUILDERS_VERSION}",
                config.version
            ),
        );
        return diagnostics;
    }
    if config.builders.len() > 64 {
        diagnostic(
            &mut diagnostics,
            "too_many_repository_builders",
            "builders",
            "a repository may define at most 64 builders",
        );
    }
    let mut keys = HashSet::new();
    for (index, builder) in config.builders.iter().enumerate() {
        if !valid_key(&builder.key, 64) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_builder_key",
                format!("builders[{index}].key"),
                "builder keys must be lowercase and at most 64 characters",
            );
        } else if !keys.insert(&builder.key) {
            diagnostic(
                &mut diagnostics,
                "duplicate_repository_builder_key",
                format!("builders[{index}].key"),
                "builder keys must be unique within a repository",
            );
        }
        if builder.display_name.trim().is_empty() || builder.display_name.len() > 200 {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_builder_display_name",
                format!("builders[{index}].display_name"),
                "display names must contain 1 to 200 characters",
            );
        }
        if !valid_repository_builder_path(&builder.oci.dockerfile, false) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_builder_dockerfile",
                format!("builders[{index}].oci.dockerfile"),
                "dockerfile must be a safe repository-relative path",
            );
        }
        if !valid_repository_builder_path(&builder.oci.context, true) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_builder_context",
                format!("builders[{index}].oci.context"),
                "context must be a safe repository-relative path or .",
            );
        }
        if !valid_key(&builder.oci.base, 64) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_builder_base",
                format!("builders[{index}].oci.base"),
                "base must be a lowercase approved platform builder key",
            );
        }
    }
    diagnostics
}

fn valid_repository_builder_path(value: &str, permit_current_directory: bool) -> bool {
    if permit_current_directory && value == "." {
        return true;
    }
    valid_relative_path(value)
}

// Keeping the ordered checks together preserves stable diagnostic order.
#[allow(clippy::too_many_lines)]
fn validate(config: &AgentConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != REUSABLE_RELEASE_VERSION {
        diagnostic(
            &mut diagnostics,
            "unsupported_version",
            "version",
            format!(
                "configuration version {} is unsupported; expected {REUSABLE_RELEASE_VERSION}",
                config.version
            ),
        );
        return diagnostics;
    }
    if config.agent.name.trim().is_empty() || config.agent.name.len() > 128 {
        diagnostic(
            &mut diagnostics,
            "invalid_agent_name",
            "agent.name",
            "name must contain 1 to 128 characters",
        );
    }
    validate_v2(config, &mut diagnostics);
    if !(1..=64).contains(&config.resources.vcpus) {
        diagnostic(
            &mut diagnostics,
            "invalid_vcpus",
            "resources.vcpus",
            "vcpus must be between 1 and 64",
        );
    }
    if !(128..=1_048_576).contains(&config.resources.memory_mib) {
        diagnostic(
            &mut diagnostics,
            "invalid_memory",
            "resources.memory_mib",
            "memory_mib must be between 128 and 1048576",
        );
    }
    if config.root_image.reference.trim().is_empty() {
        diagnostic(
            &mut diagnostics,
            "missing_root_image",
            "root_image.reference",
            "root image reference must not be empty",
        );
    }
    if config.workspace.mount {
        validate_absolute_path(
            &mut diagnostics,
            "workspace.path",
            &config.workspace.path,
            "invalid_workspace_path",
        );
    }
    if config.results.declared_files.len() > 128 {
        diagnostic(
            &mut diagnostics,
            "too_many_declared_files",
            "results.declared_files",
            "at most 128 result files may be declared",
        );
    }
    let mut declared_files = HashSet::new();
    for (index, result_path) in config.results.declared_files.iter().enumerate() {
        let path = Path::new(result_path);
        let valid = !path.is_absolute()
            && path.components().next().is_some()
            && path.components().all(|component| {
                matches!(component, std::path::Component::Normal(value)
                    if !value.eq_ignore_ascii_case(std::ffi::OsStr::new(".git")))
            });
        if !valid {
            diagnostic(
                &mut diagnostics,
                "invalid_declared_file",
                format!("results.declared_files[{index}]"),
                "declared file paths must be relative, traversal-free, and outside .git",
            );
        } else if !declared_files.insert(result_path) {
            diagnostic(
                &mut diagnostics,
                "duplicate_declared_file",
                format!("results.declared_files[{index}]"),
                "declared file paths must be unique",
            );
        }
    }
    for (index, pattern) in config.triggers.refs.iter().enumerate() {
        let value = pattern.strip_suffix("/*").unwrap_or(pattern);
        if GitRef::parse(value.to_owned()).is_err() {
            diagnostic(
                &mut diagnostics,
                "invalid_trigger_ref",
                format!("triggers.refs[{index}]"),
                "trigger must be a fully-qualified ref or end in /*",
            );
        }
    }
    diagnostics
}

#[allow(clippy::too_many_lines)]
fn validate_v2(config: &AgentConfig, diagnostics: &mut Vec<Diagnostic>) {
    if config
        .agent
        .key
        .as_deref()
        .is_none_or(|key| !valid_key(key, 64))
    {
        diagnostic(
            diagnostics,
            "invalid_agent_key",
            "agent.key",
            "version 2 requires a stable lowercase key of at most 64 characters",
        );
    }
    validate_relative_path(
        diagnostics,
        "guest.command",
        &config.guest.command,
        "invalid_release_command",
    );
    validate_relative_path(
        diagnostics,
        "guest.working_directory",
        &config.guest.working_directory,
        "invalid_release_working_directory",
    );
    if !valid_digest(&config.root_image.reference) {
        diagnostic(
            diagnostics,
            "unpinned_root_image",
            "root_image.reference",
            "version 2 runtime root images must end in a lowercase sha256 digest",
        );
    }
    let Some(build) = &config.build else {
        diagnostic(
            diagnostics,
            "missing_build",
            "build",
            "version 2 requires an isolated build definition",
        );
        return;
    };
    validate_absolute_path(
        diagnostics,
        "build.command",
        &build.command,
        "invalid_build_command",
    );
    validate_absolute_path(
        diagnostics,
        "build.working_directory",
        &build.working_directory,
        "invalid_build_working_directory",
    );
    match (&build.root_image, &build.builder) {
        (Some(root_image), None) if !valid_digest(root_image) => diagnostic(
            diagnostics,
            "unpinned_build_root_image",
            "build.root_image",
            "build root image must end in a lowercase sha256 digest",
        ),
        (Some(_), None) => {}
        (Some(_), Some(_)) => diagnostic(
            diagnostics,
            "ambiguous_build_builder",
            "build",
            "configure either build.root_image or build.builder, not both",
        ),
        (None, None) => diagnostic(
            diagnostics,
            "missing_build_builder",
            "build",
            "configure a digest-pinned build.root_image or a catalog build.builder",
        ),
        (None, Some(BuilderSelection::Platform { key })) => {
            if !valid_key(key, 64) {
                diagnostic(
                    diagnostics,
                    "invalid_builder_key",
                    "build.builder.key",
                    "platform builder keys must be lowercase and at most 64 characters",
                );
            }
        }
        (None, Some(BuilderSelection::Project { id })) => {
            if uuid::Uuid::parse_str(id).is_err() {
                diagnostic(
                    diagnostics,
                    "invalid_project_builder_id",
                    "build.builder.id",
                    "project builder id must be a UUID",
                );
            }
        }
        (None, Some(BuilderSelection::Repository { key })) => {
            if !valid_key(key, 64) {
                diagnostic(
                    diagnostics,
                    "invalid_repository_builder_key",
                    "build.builder.key",
                    "repository builder keys must be lowercase and at most 64 characters",
                );
            }
        }
    }
    if build.artifacts.is_empty() || build.artifacts.len() > 128 {
        diagnostic(
            diagnostics,
            "invalid_build_artifact_count",
            "build.artifacts",
            "build must declare between 1 and 128 outputs",
        );
    }
    let mut paths = HashSet::new();
    for (index, artifact) in build.artifacts.iter().enumerate() {
        if !valid_relative_path(&artifact.path) {
            diagnostic(
                diagnostics,
                "invalid_build_artifact_path",
                format!("build.artifacts[{index}].path"),
                "artifact path must be relative, traversal-free, and outside .git",
            );
        } else if !paths.insert(&artifact.path) {
            diagnostic(
                diagnostics,
                "duplicate_build_artifact_path",
                format!("build.artifacts[{index}].path"),
                "artifact paths must be unique",
            );
        }
    }
    for (index, trigger) in build.triggers.iter().enumerate() {
        let value = trigger.strip_suffix("/*").unwrap_or(trigger);
        if GitRef::parse(value.to_owned()).is_err() {
            diagnostic(
                diagnostics,
                "invalid_build_trigger",
                format!("build.triggers[{index}]"),
                "build trigger must be an exact ref or terminal prefix",
            );
        }
    }
    validate_parameters(&config.parameters, diagnostics);
    validate_secret_slots(&config.secret_slots, diagnostics);
    if let Some(hook) = &config.update_hook {
        validate_relative_path(
            diagnostics,
            "update_hook.command",
            &hook.command,
            "invalid_update_hook_command",
        );
        if !(1..=86_400).contains(&hook.timeout_seconds) {
            diagnostic(
                diagnostics,
                "invalid_update_hook_timeout",
                "update_hook.timeout_seconds",
                "update hook timeout must be between 1 and 86400 seconds",
            );
        }
    }
}

fn validate_parameters(parameters: &[ParameterDeclaration], diagnostics: &mut Vec<Diagnostic>) {
    let mut names = HashSet::new();
    for (index, parameter) in parameters.iter().enumerate() {
        if !valid_key(&parameter.name, 64) || parameter.name.starts_with("hephaestus_") {
            diagnostic(
                diagnostics,
                "invalid_parameter_name",
                format!("parameters[{index}].name"),
                "parameter name is malformed or reserved",
            );
        } else if !names.insert(&parameter.name) {
            diagnostic(
                diagnostics,
                "duplicate_parameter",
                format!("parameters[{index}].name"),
                "parameter names must be unique",
            );
        }
        let schema_valid = match &parameter.value_type {
            ParameterType::String {
                minimum_length,
                maximum_length,
            } => minimum_length <= maximum_length && *maximum_length <= 4096,
            ParameterType::Integer { minimum, maximum } => minimum <= maximum,
            ParameterType::Boolean => true,
            ParameterType::Enum { values } => {
                (1..=64).contains(&values.len())
                    && values.iter().all(|value| (1..=128).contains(&value.len()))
                    && values.iter().collect::<HashSet<_>>().len() == values.len()
            }
        };
        if !schema_valid {
            diagnostic(
                diagnostics,
                "invalid_parameter_schema",
                format!("parameters[{index}]"),
                "parameter type must have explicit valid bounds and unique choices",
            );
        }
        if parameter.required && parameter.default.is_none() {
            continue;
        }
        if let Some(default) = &parameter.default
            && !parameter_default_matches(&parameter.value_type, default)
        {
            diagnostic(
                diagnostics,
                "invalid_parameter_default",
                format!("parameters[{index}].default"),
                "parameter default does not match its declared type and bounds",
            );
        }
    }
}

fn parameter_default_matches(value_type: &ParameterType, value: &ParameterDefault) -> bool {
    match (value_type, value) {
        (
            ParameterType::String {
                minimum_length,
                maximum_length,
            },
            ParameterDefault::String(value),
        ) => (usize::from(*minimum_length)..=usize::from(*maximum_length))
            .contains(&value.chars().count()),
        (ParameterType::Integer { minimum, maximum }, ParameterDefault::Integer(value)) => {
            (*minimum..=*maximum).contains(value)
        }
        (ParameterType::Boolean, ParameterDefault::Boolean(_)) => true,
        (ParameterType::Enum { values }, ParameterDefault::String(value)) => values.contains(value),
        _ => false,
    }
}

fn validate_secret_slots(slots: &[SecretSlotDeclaration], diagnostics: &mut Vec<Diagnostic>) {
    let mut keys = HashSet::new();
    for (index, slot) in slots.iter().enumerate() {
        if !valid_key(&slot.key, 64) {
            diagnostic(
                diagnostics,
                "invalid_secret_slot_key",
                format!("secret_slots[{index}].key"),
                "secret slot key must be a bounded lowercase identifier",
            );
        } else if !keys.insert(&slot.key) {
            diagnostic(
                diagnostics,
                "duplicate_secret_slot",
                format!("secret_slots[{index}].key"),
                "secret slot keys must be unique",
            );
        }
        if slot.purpose.trim().is_empty() || slot.purpose.len() > 512 {
            diagnostic(
                diagnostics,
                "invalid_secret_slot_purpose",
                format!("secret_slots[{index}].purpose"),
                "secret slot purpose must contain 1 to 512 characters",
            );
        }
        if slot.delivery_modes.is_empty()
            || slot.delivery_modes.len() > 2
            || slot.delivery_modes.iter().collect::<HashSet<_>>().len() != slot.delivery_modes.len()
            || slot.phases.is_empty()
            || slot.phases.len() > 2
            || slot.phases.iter().collect::<HashSet<_>>().len() != slot.phases.len()
            || slot.destinations.len() > 32
        {
            diagnostic(
                diagnostics,
                "invalid_secret_slot_policy",
                format!("secret_slots[{index}]"),
                "secret slot modes, phases, or destinations are empty, duplicated, or oversized",
            );
        }
    }
}

fn valid_key(value: &str, maximum: usize) -> bool {
    (1..=maximum).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || ((byte == b'_' || byte == b'-') && index > 0)
        })
}

fn valid_digest(value: &str) -> bool {
    value.rsplit_once("@sha256:").is_some_and(|(_, digest)| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn validate_relative_path(diagnostics: &mut Vec<Diagnostic>, path: &str, value: &str, code: &str) {
    if !valid_relative_path(value) {
        diagnostic(
            diagnostics,
            code,
            path,
            "path must be relative, traversal-free, and outside .git",
        );
    }
}

fn valid_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && value.len() <= 1024
        && !path.is_absolute()
        && !value.contains('\\')
        && path.components().all(|component| {
            matches!(component, std::path::Component::Normal(part)
                if !part.eq_ignore_ascii_case(std::ffi::OsStr::new(".git")))
        })
}

fn validate_absolute_path(diagnostics: &mut Vec<Diagnostic>, path: &str, value: &str, code: &str) {
    let parsed = Path::new(value);
    if !parsed.is_absolute()
        || parsed
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        diagnostic(
            diagnostics,
            code,
            path,
            "path must be absolute and must not contain parent traversal",
        );
    }
}

fn diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    code: impl Into<String>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    diagnostics.push(Diagnostic {
        code: code.into(),
        path: Some(path.into()),
        message: message.into(),
    });
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{
        BuilderSelection, REPOSITORY_BUILDERS_VERSION, REUSABLE_RELEASE_VERSION, parse,
        parse_repository_builders,
    };
    use forge_domain::GitRef;

    const VALID: &str = r#"
version = 2

[agent]
name = "reviewer"
key = "reviewer"

[build]
command = "/usr/bin/build"
working_directory = "/workspace/source"
root_image = "registry.example/build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
triggers = ["refs/heads/main"]

[build.resources]
vcpus = 1
memory_mib = 512

[build.network]
profile = "disabled"

[[build.artifacts]]
path = "bin/review"
kind = "executable"

[guest]
command = "bin/review"
arguments = ["--format=json"]
working_directory = "bin"

[resources]
vcpus = 2
memory_mib = 512

[root_image]
reference = "registry.example/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[workspace]
mount = true
path = "/workspace/repo"
read_only = true

[state_volume]
enabled = true

[results]
declared_files = ["reports/review.json"]

[network]
profile = "disabled"

[triggers]
push = true
refs = ["refs/heads/*"]
"#;

    #[test]
    fn parses_and_matches_valid_release() {
        let parsed = parse(VALID.as_bytes());
        let config = parsed.config.expect("valid config");
        assert_eq!(config.version, REUSABLE_RELEASE_VERSION);
        assert!(parsed.diagnostics.is_empty());
        assert!(
            config
                .triggers
                .matches(&GitRef::parse("refs/heads/main").expect("valid ref"))
        );
        assert!(
            !config
                .triggers
                .matches(&GitRef::parse("refs/tags/v1").expect("valid ref"))
        );
        assert_eq!(parsed.hash.as_str().len(), 64);
        assert_eq!(
            parsed
                .normalized_hash
                .expect("valid config has normalized hash")
                .as_str()
                .len(),
            64
        );
    }

    #[test]
    fn reports_version_and_field_diagnostics() {
        let invalid = VALID.replace("version = 2", "version = 99");
        let parsed = parse(invalid.as_bytes());
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].code, "unsupported_version");
    }

    #[test]
    fn reports_syntax_errors_without_panicking() {
        let parsed = parse(b"version = [");
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
    }

    #[test]
    fn rejects_unsafe_and_duplicate_declared_results() {
        let invalid = VALID.replace(
            r#"declared_files = ["reports/review.json"]"#,
            r#"declared_files = ["../escape", "report.json", "report.json"]"#,
        );
        let parsed = parse(invalid.as_bytes());
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics[0].code, "invalid_declared_file");
        assert_eq!(parsed.diagnostics[1].code, "duplicate_declared_file");
    }

    #[test]
    fn parses_reusable_release_contract_and_symbolic_slots() {
        let digest = "a".repeat(64);
        let source = format!(
            r#"
version = {REUSABLE_RELEASE_VERSION}

[agent]
name = "Reviewer"
key = "reviewer"

[build]
command = "/bin/build"
arguments = ["--release"]
working_directory = "/workspace/source"
root_image = "build.example/image@sha256:{digest}"
triggers = ["refs/heads/main"]

[build.resources]
vcpus = 2
memory_mib = 1024

[build.network]
profile = "disabled"

[[build.artifacts]]
path = "bin/reviewer"
kind = "executable"
media_type = "application/octet-stream"

[guest]
command = "bin/reviewer"
arguments = ["--json"]
working_directory = "bin"

[resources]
vcpus = 4
memory_mib = 2048

[root_image]
reference = "runtime.example/image@sha256:{digest}"

[workspace]
mount = true
path = "/workspace/repo"
read_only = true

[state_volume]
enabled = true

[network]
profile = "broker_only"

[triggers]
push = false
refs = []

[[parameters]]
name = "severity"
type = "enum"
values = ["warning", "error"]
required = false
default = "warning"

[[parameters]]
name = "max_findings"
type = "integer"
minimum = 1
maximum = 100
required = true

[[secret_slots]]
key = "model"
purpose = "Call the configured model provider"
required = true
delivery_modes = ["brokered"]
phases = ["normal", "update"]
destinations = ["api.model.example"]

[update_hook]
command = "bin/migrate"
arguments = ["--transactional"]
timeout_seconds = 300

[update_hook.resources]
vcpus = 1
memory_mib = 512
"#
        );
        let parsed = parse(source.as_bytes());
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            parsed.diagnostics
        );
        let config = parsed.config.expect("version 2 should parse");
        assert_eq!(config.version, REUSABLE_RELEASE_VERSION);
        assert_eq!(config.agent.key.as_deref(), Some("reviewer"));
        assert_eq!(config.parameters.len(), 2);
        assert_eq!(config.secret_slots.len(), 1);
        assert!(config.update_hook.is_some());

        let second = parse(source.as_bytes());
        assert_eq!(parsed.normalized_hash, second.normalized_hash);
    }

    #[test]
    fn rejects_tenant_secret_material_and_unsafe_release_paths() {
        let digest = "a".repeat(64);
        let source = format!(
            r#"
version = 2
[agent]
name = "Reviewer"
key = "reviewer"
[build]
command = "/bin/build"
working_directory = "/source"
root_image = "image@sha256:{digest}"
[build.resources]
vcpus = 1
memory_mib = 512
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "../escape"
kind = "file"
[guest]
command = "/host/path"
working_directory = "../source"
[resources]
vcpus = 1
memory_mib = 512
[root_image]
reference = "image@sha256:{digest}"
[workspace]
mount = true
path = "/workspace/repo"
[state_volume]
enabled = false
[network]
profile = "disabled"
[triggers]
push = false
[[secret_slots]]
key = "model"
purpose = "model"
delivery_modes = ["brokered"]
phases = ["normal"]
secret_id = "f774d581-c89e-4420-9712-24cc642d2a9a"
plaintext = "must-never-be-accepted"
"#
        );
        let parsed = parse(source.as_bytes());
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
        assert!(
            !format!("{:?}", parsed.diagnostics).contains("must-never-be-accepted"),
            "parser diagnostics must not echo rejected plaintext"
        );
    }

    #[test]
    fn parses_repository_builder_manifest() {
        let manifest = format!(
            r#"
version = {REPOSITORY_BUILDERS_VERSION}

[[builders]]
key = "typescript-tools"
display_name = "TypeScript tools"

[builders.oci]
dockerfile = "containers/typescript-tools.Dockerfile"
context = "."
base = "typescript-node-ubuntu"
"#
        );
        let parsed = parse_repository_builders(manifest.as_bytes());
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let builders = parsed.config.expect("valid repository builders");
        assert_eq!(builders.builders.len(), 1);
        assert_eq!(builders.builders[0].oci.base, "typescript-node-ubuntu");
    }

    #[test]
    fn parses_repository_builder_selection_without_changing_runtime_root() {
        let build_root = "root_image = \"registry.example/build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"";
        let source = VALID.replace(
            build_root,
            "builder = { kind = \"repository\", key = \"typescript-tools\" }",
        );
        let parsed = parse(source.as_bytes());
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let config = parsed.config.expect("valid repository builder selection");
        assert!(matches!(
            config.build.as_ref().and_then(|build| build.builder.as_ref()),
            Some(BuilderSelection::Repository { key }) if key == "typescript-tools"
        ));
        assert_eq!(
            config.root_image.reference,
            "registry.example/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn rejects_unsafe_repository_builder_manifest_paths_and_duplicate_keys() {
        let manifest = r#"
version = 1

[[builders]]
key = "typescript-tools"
display_name = "TypeScript tools"
[builders.oci]
dockerfile = "../Dockerfile"
context = "."
base = "typescript-node-ubuntu"

[[builders]]
key = "typescript-tools"
display_name = "Duplicate"
[builders.oci]
dockerfile = "Dockerfile"
context = "../../host"
base = "node:latest"
"#;
        let parsed = parse_repository_builders(manifest.as_bytes());
        assert!(parsed.config.is_none());
        let codes = parsed
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            codes,
            [
                "invalid_repository_builder_dockerfile",
                "duplicate_repository_builder_key",
                "invalid_repository_builder_context",
                "invalid_repository_builder_base",
            ]
        );
    }
}

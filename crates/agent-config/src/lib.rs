//! Versioned `agent.toml` parsing, validation, and trigger matching.

use capability_domain::{
    CapabilityError, CapabilityOperation, CapabilityRequirement, CapabilityRequirementId,
    CapabilityResourceKind, CapabilitySlotKey,
};
use forge_domain::GitRef;
use git_capability_domain::{
    BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitCapabilityError, GitOperation, RefGlob, RefMutationPermission,
    RefNamespacePolicy, RefUpdatePolicy, TransferLimits,
};
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

/// A declarative OCI image identity resolved by the catalog before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSelection {
    /// Stable key in the OCI image catalog.
    pub key: String,
}

/// Schema version for a repository's OCI image manifest.
pub const REPOSITORY_OCI_IMAGES_VERSION: u32 = 1;

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

/// A symbolic request for one exact resource binding at instance setup.
///
/// Tenant resource identities, grants, and bearer material are deliberately
/// absent from this release-owned declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySlotDeclaration {
    /// Stable repository-scoped slot key.
    pub key: String,
    /// Human-readable, non-secret reason the released agent needs the slot.
    pub purpose: String,
    /// Compatible resource category.
    pub resource_kind: CapabilityResourceKind,
    /// Operations every binding must grant.
    #[serde(default)]
    pub required_operations: Vec<CapabilityOperation>,
    /// Operations an authorized installer may elect to grant.
    #[serde(default)]
    pub optional_operations: Vec<CapabilityOperation>,
    /// Whether the instance revision must bind this slot before it can run.
    #[serde(default)]
    pub required: bool,
    /// Optional typed Git authority ceiling. This is valid only for a
    /// repository slot carrying Git transport operations.
    #[serde(default)]
    pub git: Option<GitCapabilityDeclaration>,
}

impl CapabilitySlotDeclaration {
    /// Converts release-owned syntax into the provider-neutral normalized
    /// domain contract using a publication-assigned requirement identity.
    ///
    /// # Errors
    ///
    /// Returns a capability validation error when the slot key, operation
    /// sets, or resource-operation pairing is invalid.
    pub fn to_requirement(
        &self,
        id: CapabilityRequirementId,
    ) -> Result<CapabilityRequirement, CapabilityError> {
        CapabilityRequirement::new(
            id,
            CapabilitySlotKey::parse(self.key.clone())?,
            self.resource_kind,
            self.required_operations.iter().copied(),
            self.optional_operations.iter().copied(),
            self.required,
        )
    }

    /// Converts the optional repository Git declaration into its normalized
    /// release ceiling.
    ///
    /// # Errors
    ///
    /// Returns an error for broad patterns without their explicit opt-in,
    /// malformed patterns, invalid transport limits, or conflicting generic
    /// operations and Git receive rules.
    pub fn git_ceiling(&self) -> Result<Option<GitCapabilityCeiling>, GitCapabilityError> {
        self.git
            .as_ref()
            .map(|declaration| declaration.to_ceiling(self))
            .transpose()
    }
}

/// Release-owned typed Git authority below a repository capability slot.
// Explicit booleans keep TOML transition rules readable and independently
// attenuable; combining them into bitsets would obscure the security contract.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitCapabilityDeclaration {
    /// Fully-qualified visible/writable ref patterns.
    pub ref_globs: Vec<String>,
    /// Repository-relative changed-path patterns required for receive.
    #[serde(default)]
    pub changed_path_globs: Vec<String>,
    /// Explicit opt-in for whole-namespace ref patterns such as
    /// `refs/heads/**`.
    #[serde(default)]
    pub allow_broad_ref_globs: bool,
    /// Explicit opt-in for repository-wide path patterns.
    #[serde(default)]
    pub allow_broad_changed_path_globs: bool,
    /// Existing branch update rule.
    #[serde(default)]
    pub branch_updates: GitBranchUpdateDeclaration,
    /// Whether branch creation is allowed.
    #[serde(default)]
    pub create_branches: bool,
    /// Whether branch deletion is allowed.
    #[serde(default)]
    pub delete_branches: bool,
    /// Whether tag creation is allowed.
    #[serde(default)]
    pub create_tags: bool,
    /// Whether existing tag updates are allowed.
    #[serde(default)]
    pub update_tags: bool,
    /// Whether tag deletion is allowed.
    #[serde(default)]
    pub delete_tags: bool,
    /// Whether creation in other explicit ref namespaces is allowed.
    #[serde(default)]
    pub create_other_refs: bool,
    /// Whether updates in other explicit ref namespaces are allowed.
    #[serde(default)]
    pub update_other_refs: bool,
    /// Whether deletion in other explicit ref namespaces is allowed.
    #[serde(default)]
    pub delete_other_refs: bool,
    /// Explicit bounded transfer limits.
    pub transfer: GitTransferLimitDeclaration,
    /// Require dispatch to bind the triggering commit as exact old parent.
    #[serde(default)]
    pub exact_parent_required: bool,
}

impl GitCapabilityDeclaration {
    fn to_ceiling(
        &self,
        slot: &CapabilitySlotDeclaration,
    ) -> Result<GitCapabilityCeiling, GitCapabilityError> {
        let declares = |operation| {
            slot.required_operations.contains(&operation)
                || slot.optional_operations.contains(&operation)
        };
        let receive_declared = slot
            .required_operations
            .iter()
            .chain(&slot.optional_operations)
            .any(|operation| {
                matches!(
                    operation,
                    CapabilityOperation::CreateRef
                        | CapabilityOperation::UpdateRef
                        | CapabilityOperation::ForceUpdateRef
                        | CapabilityOperation::DeleteRef
                        | CapabilityOperation::CreateTag
                        | CapabilityOperation::DeleteTag
                )
            });
        if (receive_declared && !declares(CapabilityOperation::UpdateRef))
            || (self.branch_updates == GitBranchUpdateDeclaration::AllowForce
                && !declares(CapabilityOperation::ForceUpdateRef))
            || (self.create_branches && !declares(CapabilityOperation::CreateRef))
            || (self.delete_branches && !declares(CapabilityOperation::DeleteRef))
            || (self.create_tags && !declares(CapabilityOperation::CreateTag))
            || (self.update_tags && !declares(CapabilityOperation::ForceUpdateRef))
            || (self.delete_tags && !declares(CapabilityOperation::DeleteTag))
            || (self.create_other_refs && !declares(CapabilityOperation::CreateRef))
            || (self.update_other_refs && !declares(CapabilityOperation::ForceUpdateRef))
            || (self.delete_other_refs && !declares(CapabilityOperation::DeleteRef))
        {
            return Err(GitCapabilityError::ConflictingScope(
                "Git transition rules exceed the repository operation ceiling",
            ));
        }
        let parse_ref = |value: &String| {
            if self.allow_broad_ref_globs {
                RefGlob::parse_explicitly_broad(value.clone())
            } else {
                RefGlob::parse(value.clone())
            }
        };
        let parse_path = |value: &String| {
            if self.allow_broad_changed_path_globs {
                ChangedPathGlob::parse_explicitly_broad(value.clone())
            } else {
                ChangedPathGlob::parse(value.clone())
            }
        };
        let operations = git_operations(
            slot.required_operations
                .iter()
                .chain(&slot.optional_operations)
                .copied(),
        );
        GitCapabilityCeiling::new(GitCapabilityCeilingInput {
            operations,
            ref_globs: self
                .ref_globs
                .iter()
                .map(parse_ref)
                .collect::<Result<_, _>>()?,
            changed_path_globs: self
                .changed_path_globs
                .iter()
                .map(parse_path)
                .collect::<Result<_, _>>()?,
            update_policy: RefUpdatePolicy {
                branches: BranchRefPolicy {
                    updates: self.branch_updates.into(),
                    create: permission(self.create_branches),
                    delete: permission(self.delete_branches),
                },
                tags: RefNamespacePolicy {
                    create: permission(self.create_tags),
                    update: permission(self.update_tags),
                    delete: permission(self.delete_tags),
                },
                other: RefNamespacePolicy {
                    create: permission(self.create_other_refs),
                    update: permission(self.update_other_refs),
                    delete: permission(self.delete_other_refs),
                },
            },
            transfer_limits: TransferLimits::new(
                self.transfer.request_bytes,
                self.transfer.pack_bytes,
                self.transfer.object_count,
                self.transfer.ref_updates,
            )?,
            exact_parent_required: self.exact_parent_required,
        })
    }
}

/// Existing branch transition allowed by a Git ceiling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitBranchUpdateDeclaration {
    /// Existing branches may only fast-forward.
    #[default]
    FastForwardOnly,
    /// Existing branches may be updated non-fast-forward.
    AllowForce,
}

impl From<GitBranchUpdateDeclaration> for BranchUpdatePolicy {
    fn from(value: GitBranchUpdateDeclaration) -> Self {
        match value {
            GitBranchUpdateDeclaration::FastForwardOnly => Self::FastForwardOnly,
            GitBranchUpdateDeclaration::AllowForce => Self::AllowForce,
        }
    }
}

/// Required hard limits for one Git smart-HTTP request/receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitTransferLimitDeclaration {
    /// Maximum encoded request bytes.
    pub request_bytes: u64,
    /// Maximum accepted pack bytes.
    pub pack_bytes: u64,
    /// Maximum accepted object count.
    pub object_count: u32,
    /// Maximum atomic ref updates.
    pub ref_updates: u16,
}

const fn permission(allowed: bool) -> RefMutationPermission {
    if allowed {
        RefMutationPermission::Allow
    } else {
        RefMutationPermission::Deny
    }
}

fn git_operations(operations: impl IntoIterator<Item = CapabilityOperation>) -> Vec<GitOperation> {
    let mut read = false;
    let mut receive = false;
    for operation in operations {
        match operation {
            CapabilityOperation::GitRead => read = true,
            CapabilityOperation::CreateRef
            | CapabilityOperation::UpdateRef
            | CapabilityOperation::ForceUpdateRef
            | CapabilityOperation::DeleteRef
            | CapabilityOperation::CreateTag
            | CapabilityOperation::DeleteTag => receive = true,
            _ => {}
        }
    }
    let mut result = Vec::with_capacity(3);
    if read {
        result.extend([GitOperation::Discover, GitOperation::Fetch]);
    }
    if receive {
        result.push(GitOperation::Receive);
    }
    result
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

/// Result of parsing one repository `heph.images.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRepositoryOciImages {
    /// Hash of the exact source bytes.
    pub hash: ConfigHash,
    /// Validated manifest, absent on failure.
    pub config: Option<RepositoryOciImagesConfig>,
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
        let normalized = normalized_config(config.clone());
        toml::to_string(&normalized).map_or_else(
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

/// Parses and validates a repository's root-level `heph.images.toml`.
///
/// This function only validates the declarative source contract. The receive
/// workflow verifies the declared paths against the exact Git tree and resolves
/// `oci.base` to an approved digest-pinned platform image transactionally.
#[must_use]
pub fn parse_repository_oci_images(source: &[u8]) -> ParsedRepositoryOciImages {
    let source_hash = hash(source);
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            return ParsedRepositoryOciImages {
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
    let config = match toml::from_str::<RepositoryOciImagesConfig>(text) {
        Ok(config) => config,
        Err(error) => {
            return ParsedRepositoryOciImages {
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
    let diagnostics = validate_repository_oci_images(&config);
    ParsedRepositoryOciImages {
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

fn normalized_config(mut config: AgentConfig) -> AgentConfig {
    for declaration in &mut config.capability_slots {
        declaration.required_operations.sort_unstable();
        declaration.optional_operations.sort_unstable();
    }
    config
        .capability_slots
        .sort_unstable_by(|left, right| left.key.cmp(&right.key));
    config
}

fn validate_repository_oci_images(config: &RepositoryOciImagesConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != REPOSITORY_OCI_IMAGES_VERSION {
        diagnostic(
            &mut diagnostics,
            "unsupported_repository_oci_images_version",
            "version",
            format!(
                "repository OCI image version {} is unsupported; expected {REPOSITORY_OCI_IMAGES_VERSION}",
                config.version
            ),
        );
        return diagnostics;
    }
    if config.images.len() > 64 {
        diagnostic(
            &mut diagnostics,
            "too_many_repository_oci_images",
            "images",
            "a repository may define at most 64 OCI images",
        );
    }
    let mut keys = HashSet::new();
    for (index, image) in config.images.iter().enumerate() {
        if !valid_key(&image.key, 64) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_key",
                format!("images[{index}].key"),
                "OCI image keys must be lowercase and at most 64 characters",
            );
        } else if !keys.insert(&image.key) {
            diagnostic(
                &mut diagnostics,
                "duplicate_repository_oci_image_key",
                format!("images[{index}].key"),
                "OCI image keys must be unique within a repository",
            );
        }
        if image.display_name.trim().is_empty() || image.display_name.len() > 200 {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_display_name",
                format!("images[{index}].display_name"),
                "display names must contain 1 to 200 characters",
            );
        }
        if !valid_repository_oci_image_path(&image.build.dockerfile, false) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_dockerfile",
                format!("images[{index}].build.dockerfile"),
                "dockerfile must be a safe repository-relative path",
            );
        }
        if !valid_repository_oci_image_path(&image.build.context, true) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_context",
                format!("images[{index}].build.context"),
                "context must be a safe repository-relative path or .",
            );
        }
        if !valid_key(&image.build.base.key, 64) {
            diagnostic(
                &mut diagnostics,
                "invalid_repository_oci_image_base",
                format!("images[{index}].build.base.key"),
                "base image keys must be lowercase and at most 64 characters",
            );
        }
    }
    diagnostics
}

fn valid_repository_oci_image_path(value: &str, permit_current_directory: bool) -> bool {
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
    if !valid_key(&config.guest.image.key, 64) {
        diagnostic(
            &mut diagnostics,
            "invalid_guest_image_key",
            "guest.image.key",
            "guest image keys must be lowercase and at most 64 characters",
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
    if config.publication.mode == PublicationMode::Proposal
        && config.workspace.mount
        && !config.workspace.read_only
    {
        diagnostic(
            &mut diagnostics,
            "proposal_workspace_must_be_read_only",
            "workspace.read_only",
            "proposal mode exposes only a read-only source tree and never a Git write remote",
        );
    }
    if config.publication.mode == PublicationMode::RuntimeGit && config.workspace.mount {
        diagnostic(
            &mut diagnostics,
            "runtime_git_uses_capability_worktrees",
            "workspace.mount",
            "runtime_git mode cannot infer repository access from the legacy proposal workspace",
        );
    }
    match (
        config.publication.mode,
        config.publication.repository_slot.as_ref(),
    ) {
        (PublicationMode::Proposal, Some(_)) => diagnostic(
            &mut diagnostics,
            "proposal_repository_slot_forbidden",
            "publication.repository_slot",
            "proposal mode does not select a runtime repository capability",
        ),
        (PublicationMode::RuntimeGit, None) => diagnostic(
            &mut diagnostics,
            "runtime_git_repository_slot_required",
            "publication.repository_slot",
            "runtime_git mode requires an explicit repository capability slot",
        ),
        (PublicationMode::RuntimeGit, Some(slot)) => {
            let declaration = config
                .capability_slots
                .iter()
                .find(|declaration| declaration.key == slot.as_str());
            if declaration.is_none_or(|declaration| {
                declaration.resource_kind != CapabilityResourceKind::Repository
                    || !declaration.required
                    || declaration.git.is_none()
            }) {
                diagnostic(
                    &mut diagnostics,
                    "runtime_git_repository_slot_invalid",
                    "publication.repository_slot",
                    "runtime_git repository_slot must name a required repository capability slot with a typed Git ceiling",
                );
            }
        }
        (PublicationMode::Proposal, None) => {}
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
    if !valid_key(&config.guest.image.key, 64) {
        diagnostic(
            diagnostics,
            "invalid_guest_image_key",
            "guest.image.key",
            "guest image keys must be lowercase and at most 64 characters",
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
    if !valid_key(&build.image.key, 64) {
        diagnostic(
            diagnostics,
            "invalid_build_image_key",
            "build.image.key",
            "build image keys must be lowercase and at most 64 characters",
        );
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
    validate_capability_slots(&config.capability_slots, diagnostics);
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

fn validate_capability_slots(
    slots: &[CapabilitySlotDeclaration],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if slots.len() > 64 {
        diagnostic(
            diagnostics,
            "too_many_capability_slots",
            "capability_slots",
            "a release agent may declare at most 64 capability slots",
        );
    }

    let mut keys = HashSet::new();
    for (index, slot) in slots.iter().enumerate() {
        if !keys.insert(slot.key.as_str()) {
            diagnostic(
                diagnostics,
                "duplicate_capability_slot",
                format!("capability_slots[{index}].key"),
                "capability slot keys must be unique",
            );
        }
        if slot.purpose.trim().is_empty() || slot.purpose.len() > 512 {
            diagnostic(
                diagnostics,
                "invalid_capability_slot_purpose",
                format!("capability_slots[{index}].purpose"),
                "capability slot purpose must contain 1 to 512 characters",
            );
        }

        if let Err(error) =
            slot.to_requirement(CapabilityRequirementId::from_uuid(uuid::Uuid::nil()))
        {
            capability_diagnostic(diagnostics, index, slot, &error);
        }
        if slot.git.is_some() && slot.resource_kind != CapabilityResourceKind::Repository {
            diagnostic(
                diagnostics,
                "git_scope_requires_repository",
                format!("capability_slots[{index}].git"),
                "a typed Git ceiling is valid only on a repository capability slot",
            );
        } else if slot.git_ceiling().is_err() {
            diagnostic(
                diagnostics,
                "invalid_git_capability_ceiling",
                format!("capability_slots[{index}].git"),
                "Git patterns, transition rules, operations, and transfer limits must form a bounded normalized ceiling",
            );
        }
    }
}

fn capability_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    index: usize,
    slot: &CapabilitySlotDeclaration,
    error: &CapabilityError,
) {
    let base = format!("capability_slots[{index}]");
    let (code, path) = match *error {
        CapabilityError::InvalidSlotKey => ("invalid_capability_slot_key", format!("{base}.key")),
        CapabilityError::DuplicateOperation(operation) => {
            let field = if operation_occurrences(&slot.required_operations, operation) > 1 {
                "required_operations"
            } else {
                "optional_operations"
            };
            ("duplicate_capability_operation", format!("{base}.{field}"))
        }
        CapabilityError::EmptyOperationSet => ("empty_capability_operations", base),
        CapabilityError::TooManyOperations => ("too_many_capability_operations", base),
        CapabilityError::IllegalOperation { operation, .. } => {
            let field = if slot.required_operations.contains(&operation) {
                "required_operations"
            } else {
                "optional_operations"
            };
            ("illegal_capability_operation", format!("{base}.{field}"))
        }
        CapabilityError::OperationRequiredAndOptional(_) => {
            ("overlapping_capability_operations", base)
        }
        _ => ("invalid_capability_slot", base),
    };
    diagnostic(diagnostics, code, path, error.to_string());
}

fn operation_occurrences(
    operations: &[CapabilityOperation],
    expected: CapabilityOperation,
) -> usize {
    operations
        .iter()
        .filter(|operation| **operation == expected)
        .count()
}

fn valid_key(value: &str, maximum: usize) -> bool {
    (1..=maximum).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || ((byte == b'_' || byte == b'-') && index > 0)
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
        PublicationMode, REPOSITORY_OCI_IMAGES_VERSION, REUSABLE_RELEASE_VERSION, parse,
        parse_repository_oci_images,
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
image = { key = "ubuntu-native" }
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
image = { key = "ubuntu-native" }
command = "bin/review"
arguments = ["--format=json"]
working_directory = "bin"

[resources]
vcpus = 2
memory_mib = 512

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
        assert_eq!(config.publication.mode, PublicationMode::Proposal);
        assert!(!config.publication.mode.permits_git_write_remote());
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
    fn publication_mode_is_explicit_and_legacy_configs_default_to_proposal() {
        let legacy = parse(VALID.as_bytes());
        let legacy_hash = legacy.normalized_hash.clone();
        assert_eq!(
            legacy
                .config
                .expect("legacy configuration")
                .publication
                .mode,
            PublicationMode::Proposal
        );

        let explicit_proposal = VALID.replace(
            "[state_volume]",
            "[publication]\nmode = \"proposal\"\n\n[state_volume]",
        );
        assert_eq!(
            parse(explicit_proposal.as_bytes()).normalized_hash,
            legacy_hash,
            "implicit and explicit proposal mode must normalize identically"
        );

        let runtime_git = VALID.replace("mount = true", "mount = false").replace(
            "[state_volume]",
            "[publication]\nmode = \"runtime_git\"\nrepository_slot = \"content\"\n\n\
             [[capability_slots]]\nkey = \"content\"\npurpose = \"Publish content\"\n\
             resource_kind = \"repository\"\nrequired_operations = [\"git_read\"]\n\
             optional_operations = [\"update_ref\"]\nrequired = true\n\n\
             [capability_slots.git]\nref_globs = [\"refs/heads/content\"]\n\
             changed_path_globs = [\"content/**\"]\n\
             transfer = { request_bytes = 1048576, pack_bytes = 8388608, object_count = 10000, ref_updates = 8 }\n\n\
             [state_volume]",
        );
        let parsed = parse(runtime_git.as_bytes());
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            parsed.diagnostics
        );
        assert_eq!(
            parsed
                .config
                .expect("runtime Git configuration")
                .publication
                .mode,
            PublicationMode::RuntimeGit
        );
    }

    #[test]
    fn publication_modes_cannot_cross_workspace_authority_boundaries() {
        let writable_proposal = VALID.replace("read_only = true", "read_only = false");
        let parsed = parse(writable_proposal.as_bytes());
        assert_eq!(
            parsed.diagnostics[0].code,
            "proposal_workspace_must_be_read_only"
        );

        let runtime_git = VALID.replace(
            "[state_volume]",
            "[publication]\nmode = \"runtime_git\"\nrepository_slot = \"content\"\n\n\
             [[capability_slots]]\nkey = \"content\"\npurpose = \"Publish content\"\n\
             resource_kind = \"repository\"\nrequired_operations = [\"git_read\"]\n\
             optional_operations = [\"update_ref\"]\nrequired = true\n\n[state_volume]",
        );
        let parsed = parse(runtime_git.as_bytes());
        assert_eq!(
            parsed.diagnostics[0].code,
            "runtime_git_uses_capability_worktrees"
        );
    }

    #[test]
    fn runtime_git_requires_an_explicit_repository_capability_slot() {
        let missing = VALID.replace("mount = true", "mount = false").replace(
            "[state_volume]",
            "[publication]\nmode = \"runtime_git\"\n\n[state_volume]",
        );
        assert_eq!(
            parse(missing.as_bytes()).diagnostics[0].code,
            "runtime_git_repository_slot_required"
        );

        let wrong_kind = VALID.replace("mount = true", "mount = false").replace(
            "[state_volume]",
            "[publication]\nmode = \"runtime_git\"\nrepository_slot = \"state\"\n\n\
             [[capability_slots]]\nkey = \"state\"\npurpose = \"Use state\"\n\
             resource_kind = \"state_volume\"\nrequired_operations = [\"attach\"]\n\
             required = true\n\n[state_volume]",
        );
        assert_eq!(
            parse(wrong_kind.as_bytes()).diagnostics[0].code,
            "runtime_git_repository_slot_invalid"
        );

        let proposal_slot = VALID.replace(
            "[state_volume]",
            "[publication]\nmode = \"proposal\"\nrepository_slot = \"content\"\n\n\
             [state_volume]",
        );
        assert_eq!(
            parse(proposal_slot.as_bytes()).diagnostics[0].code,
            "proposal_repository_slot_forbidden"
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
image = {{ key = "python-ubuntu" }}
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
image = {{ key = "python-ubuntu" }}
command = "bin/reviewer"
arguments = ["--json"]
working_directory = "bin"

[resources]
vcpus = 4
memory_mib = 2048

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
        assert!(config.capability_slots.is_empty());
        assert!(config.update_hook.is_some());

        let second = parse(source.as_bytes());
        assert_eq!(parsed.normalized_hash, second.normalized_hash);
    }

    #[test]
    fn rejects_tenant_secret_material_and_unsafe_release_paths() {
        let source = r#"
version = 2
[agent]
name = "Reviewer"
key = "reviewer"
[build]
command = "/bin/build"
working_directory = "/source"
image = { key = "ubuntu-native" }
[build.resources]
vcpus = 1
memory_mib = 512
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "../escape"
kind = "file"
[guest]
image = { key = "ubuntu-native" }
command = "/host/path"
working_directory = "../source"
[resources]
vcpus = 1
memory_mib = 512
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
"#;
        let parsed = parse(source.as_bytes());
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
        assert!(
            !format!("{:?}", parsed.diagnostics).contains("must-never-be-accepted"),
            "parser diagnostics must not echo rejected plaintext"
        );
    }

    #[test]
    fn parses_symbolic_capability_declarations() {
        let source = format!(
            r#"{VALID}

[[capability_slots]]
key = "source"
purpose = "Read source and optionally trigger its configured release"
resource_kind = "repository"
required_operations = ["git_read", "inspect"]
optional_operations = ["trigger_run"]
required = true
"#
        );
        let parsed = parse(source.as_bytes());
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            parsed.diagnostics
        );
        let config = parsed.config.expect("valid capability declaration");
        let declaration = &config.capability_slots[0];
        assert_eq!(declaration.key, "source");
        assert!(declaration.required);
        assert_eq!(declaration.required_operations.len(), 2);
        assert_eq!(declaration.optional_operations.len(), 1);
    }

    #[test]
    fn capability_hash_normalizes_slot_and_operation_order() {
        let left = format!(
            r#"{VALID}

[[capability_slots]]
key = "source"
purpose = "Source repository"
resource_kind = "repository"
required_operations = ["git_read", "inspect"]
optional_operations = ["trigger_run"]
required = true

[[capability_slots]]
key = "worker"
purpose = "Worker instance"
resource_kind = "agent_instance"
required_operations = ["execute", "inspect"]
optional_operations = ["recover", "pause"]
"#
        );
        let right = format!(
            r#"{VALID}

[[capability_slots]]
key = "worker"
purpose = "Worker instance"
resource_kind = "agent_instance"
required_operations = ["inspect", "execute"]
optional_operations = ["pause", "recover"]

[[capability_slots]]
key = "source"
purpose = "Source repository"
resource_kind = "repository"
required_operations = ["inspect", "git_read"]
optional_operations = ["trigger_run"]
required = true
"#
        );

        let left = parse(left.as_bytes());
        let right = parse(right.as_bytes());
        assert!(left.diagnostics.is_empty(), "{:?}", left.diagnostics);
        assert!(right.diagnostics.is_empty(), "{:?}", right.diagnostics);
        assert_eq!(left.normalized_hash, right.normalized_hash);
        assert_ne!(left.hash, right.hash);
    }

    #[test]
    fn rejects_duplicate_slots_and_invalid_operation_sets() {
        let cases = [
            (
                r#"
[[capability_slots]]
key = "source"
purpose = "Source"
resource_kind = "repository"
required_operations = ["inspect"]
[[capability_slots]]
key = "source"
purpose = "Duplicate"
resource_kind = "repository"
required_operations = ["git_read"]
"#,
                "duplicate_capability_slot",
            ),
            (
                r#"
[[capability_slots]]
key = "entrypoint"
purpose = "Gateway"
resource_kind = "gateway"
required_operations = ["git_read"]
"#,
                "illegal_capability_operation",
            ),
            (
                r#"
[[capability_slots]]
key = "source"
purpose = "Source"
resource_kind = "repository"
required_operations = ["inspect", "inspect"]
"#,
                "duplicate_capability_operation",
            ),
            (
                r#"
[[capability_slots]]
key = "source"
purpose = "Source"
resource_kind = "repository"
required_operations = ["inspect"]
optional_operations = ["inspect"]
"#,
                "overlapping_capability_operations",
            ),
            (
                r#"
[[capability_slots]]
key = "source"
purpose = "Source"
resource_kind = "repository"
"#,
                "empty_capability_operations",
            ),
        ];

        for (declaration, expected_code) in cases {
            let parsed = parse(format!("{VALID}\n{declaration}").as_bytes());
            assert!(parsed.config.is_none(), "case {expected_code} was accepted");
            assert!(
                parsed
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == expected_code),
                "missing {expected_code}: {:?}",
                parsed.diagnostics
            );
        }
    }

    #[test]
    fn rejects_unknown_operations_and_tenant_binding_material() {
        let malformed_operation = format!(
            r#"{VALID}
[[capability_slots]]
key = "project"
purpose = "Project"
resource_kind = "project"
required_operations = ["delete_project"]
"#
        );
        let parsed = parse(malformed_operation.as_bytes());
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics[0].code, "invalid_toml");

        for forbidden in [
            r#"resource_id = "f774d581-c89e-4420-9712-24cc642d2a9a""#,
            r#"resource_name = "production""#,
            r#"granted_operations = ["inspect"]"#,
            r#"bearer_token = "must-never-be-accepted""#,
        ] {
            let source = format!(
                r#"{VALID}
[[capability_slots]]
key = "project"
purpose = "Project"
resource_kind = "project"
required_operations = ["inspect"]
{forbidden}
"#
            );
            let parsed = parse(source.as_bytes());
            assert!(parsed.config.is_none(), "forbidden field was accepted");
            assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
            assert!(
                !format!("{:?}", parsed.diagnostics).contains("must-never-be-accepted"),
                "parser diagnostics must not echo rejected bearer material"
            );
        }
    }

    #[test]
    fn parses_repository_oci_image_manifest() {
        let manifest = format!(
            r#"
version = {REPOSITORY_OCI_IMAGES_VERSION}

[[images]]
key = "typescript-tools"
display_name = "TypeScript tools"

[images.build]
dockerfile = "containers/typescript-tools.Dockerfile"
context = "."
base = {{ key = "typescript-node-ubuntu" }}
"#
        );
        let parsed = parse_repository_oci_images(manifest.as_bytes());
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let images = parsed.config.expect("valid repository OCI images");
        assert_eq!(images.images.len(), 1);
        assert_eq!(images.images[0].build.base.key, "typescript-node-ubuntu");
    }

    #[test]
    fn parses_image_selection_for_both_execution_contexts() {
        let source = VALID.replace("ubuntu-native", "typescript-tools");
        let parsed = parse(source.as_bytes());
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let config = parsed.config.expect("valid image selection");
        assert_eq!(
            config.build.as_ref().map(|build| build.image.key.as_str()),
            Some("typescript-tools")
        );
        assert_eq!(config.guest.image.key, "typescript-tools");
    }

    #[test]
    fn rejects_repository_supplied_immutable_image_references() {
        let source = VALID.replace(
            "image = { key = \"ubuntu-native\" }",
            "image = { key = \"ubuntu-native\", reference = \"registry.example/ubuntu@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" }",
        );
        let parsed = parse(source.as_bytes());
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
    }

    #[test]
    fn rejects_unsafe_repository_oci_image_paths_and_duplicate_keys() {
        let manifest = r#"
version = 1

[[images]]
key = "typescript-tools"
display_name = "TypeScript tools"
[images.build]
dockerfile = "../Dockerfile"
context = "."
base = { key = "typescript-node-ubuntu" }

[[images]]
key = "typescript-tools"
display_name = "Duplicate"
[images.build]
dockerfile = "Dockerfile"
context = "../../host"
base = { key = "node:latest" }
"#;
        let parsed = parse_repository_oci_images(manifest.as_bytes());
        assert!(parsed.config.is_none());
        let codes = parsed
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            codes,
            [
                "invalid_repository_oci_image_dockerfile",
                "duplicate_repository_oci_image_key",
                "invalid_repository_oci_image_context",
                "invalid_repository_oci_image_base",
            ]
        );
    }
}

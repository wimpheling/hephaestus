//! Provider-neutral reusable-agent release, instance, attachment, and update
//! contracts.
//!
//! A product-level [`AgentInstance`] is a durable project-owned aggregate. It
//! is deliberately unrelated to `vm_trait::VmInstance`, which is one
//! ephemeral provider allocation used while executing a run.

use forge_domain::{CommitSha, GitRef, ProjectId, RepositoryId};
pub use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId,
};
use runtime_types::{RunId, VolumeId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, path::Path, str::FromStr};
use time::OffsetDateTime;
use uuid::Uuid;

macro_rules! identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a random version 4 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an identifier from its UUID representation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID representation.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

identifier!(
    ReleaseArtifactId,
    "A stable identifier for one immutable release artifact."
);
identifier!(
    AgentFamilyId,
    "A source-repository-scoped exported-agent family identifier."
);
identifier!(
    BuildRequestId,
    "A stable identifier for one exact idempotent build request."
);
identifier!(
    AgentUpdateId,
    "A stable identifier for one instance update transaction."
);
identifier!(
    DeferredTriggerId,
    "A stable identifier for a trigger received behind a closed run gate."
);

macro_rules! bounded_key {
    ($name:ident, $documentation:literal, $maximum:expr) => {
        #[doc = $documentation]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Parses a bounded lowercase key.
            ///
            /// # Errors
            ///
            /// Returns [`ReleaseValueError::InvalidKey`] for malformed input.
            pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
                let value = value.into();
                let valid = (1..=$maximum).contains(&value.len())
                    && value.bytes().enumerate().all(|(index, byte)| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || ((byte == b'_' || byte == b'-') && index > 0)
                    });
                if !valid {
                    return Err(ReleaseValueError::InvalidKey {
                        kind: stringify!($name),
                    });
                }
                Ok(Self(value))
            }

            /// Returns the validated value.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl TryFrom<String> for $name {
            type Error = ReleaseValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

bounded_key!(
    AgentKey,
    "A stable exported key unique within a source repository.",
    64
);
bounded_key!(
    ParameterName,
    "A stable typed-parameter name owned by a release agent.",
    64
);
bounded_key!(
    InstanceName,
    "A project-scoped reusable agent instance name.",
    128
);

/// A normalized release version selected by its source repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ReleaseVersion(String);

impl ReleaseVersion {
    /// Parses a printable version with no path or surrounding whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidVersion`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        if !(1..=128).contains(&value.len())
            || value.trim() != value
            || value.contains(['/', '\\'])
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            return Err(ReleaseValueError::InvalidVersion);
        }
        Ok(Self(value))
    }

    /// Returns the normalized version.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ReleaseVersion {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ReleaseVersion> for String {
    fn from(value: ReleaseVersion) -> Self {
        value.0
    }
}

/// A normalized relative artifact path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ArtifactPath(String);

impl ArtifactPath {
    /// Validates a path independently from any host filesystem.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidArtifactPath`] for absolute,
    /// traversal, empty, reserved, or oversized paths.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let path = Path::new(&value);
        let valid = (1..=1024).contains(&value.len())
            && !path.is_absolute()
            && !value.contains('\\')
            && path.components().all(|component| {
                matches!(
                    component,
                    std::path::Component::Normal(part)
                        if part != ".git" && !part.to_string_lossy().is_empty()
                )
            });
        if !valid {
            return Err(ReleaseValueError::InvalidArtifactPath);
        }
        Ok(Self(value))
    }

    /// Returns the normalized slash-separated path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ArtifactPath {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ArtifactPath> for String {
    fn from(value: ArtifactPath) -> Self {
        value.0
    }
}

/// An exact ref or bounded ref-prefix selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RefSelector {
    /// Match one exact fully-qualified ref.
    Exact(GitRef),
    /// Match descendants of a fully-qualified prefix.
    Prefix(GitRef),
}

impl RefSelector {
    /// Parses `refs/...` as exact or a terminal `/*` as a prefix.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidRefSelector`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        if let Some(prefix) = value.strip_suffix("/*") {
            return GitRef::parse(prefix.to_owned())
                .map(Self::Prefix)
                .map_err(|_| ReleaseValueError::InvalidRefSelector);
        }
        GitRef::parse(value)
            .map(Self::Exact)
            .map_err(|_| ReleaseValueError::InvalidRefSelector)
    }

    /// Returns whether a repository update matches this selector.
    #[must_use]
    pub fn matches(&self, git_ref: &GitRef) -> bool {
        match self {
            Self::Exact(expected) => expected == git_ref,
            Self::Prefix(prefix) => git_ref
                .as_str()
                .strip_prefix(prefix.as_str())
                .is_some_and(|suffix| suffix.starts_with('/')),
        }
    }
}

/// Durable lifecycle of a build request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildState {
    /// Accepted and awaiting execution.
    Queued,
    /// An isolated build guest is executing.
    Running,
    /// Guest stopped successfully and output is being sealed/imported.
    Importing,
    /// Complete durable build output is available.
    Succeeded,
    /// Build or safe import failed.
    Failed,
    /// Authorized cancellation completed.
    Cancelled,
}

impl BuildState {
    /// Returns whether a lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Queued, Self::Running | Self::Failed | Self::Cancelled)
                | (
                    Self::Running,
                    Self::Importing | Self::Failed | Self::Cancelled
                )
                | (Self::Importing, Self::Succeeded | Self::Failed)
        )
    }
}

/// Durable release lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseState {
    /// Complete build provenance exists but the release is mutable only through
    /// its controlled draft workflow.
    Draft,
    /// Publication permanently froze the release.
    Published,
    /// New use is revoked; historical provenance remains.
    Revoked,
}

impl ReleaseState {
    /// Returns whether a lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Published | Self::Revoked) | (Self::Published, Self::Revoked)
        )
    }
}

/// Project-owned instance lifecycle including update recovery states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    /// Normal runs may be created and dispatched.
    Active,
    /// Instance is disabled by a manager.
    Disabled,
    /// Gate is closed and pre-gate normal runs are draining.
    UpdateDraining,
    /// An isolated update hook owns the state volume.
    Updating,
    /// Candidate was explicitly rejected and the previous revision is safe.
    UpdateRejected,
    /// State compatibility is unknown after abnormal hook failure.
    PausedUnknownState,
    /// Hook committed but candidate activation needs recovery.
    PausedActivationRecovery,
    /// An authorized recovery operation is active.
    Recovering,
    /// Instance is tombstoned; history remains.
    Removed,
}

impl InstanceState {
    /// Returns whether normal requests may bind an active revision.
    #[must_use]
    pub const fn run_gate_open(self) -> bool {
        matches!(self, Self::Active | Self::UpdateRejected)
    }

    /// Returns whether a requested lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Active,
                Self::Disabled | Self::UpdateDraining | Self::Removed
            ) | (Self::Disabled, Self::Active | Self::Removed)
                | (Self::UpdateDraining, Self::Updating | Self::Active)
                | (
                    Self::Updating | Self::Recovering,
                    Self::Active
                        | Self::UpdateRejected
                        | Self::PausedUnknownState
                        | Self::PausedActivationRecovery
                )
                | (Self::UpdateRejected, Self::Active | Self::UpdateDraining)
                | (
                    Self::PausedUnknownState | Self::PausedActivationRecovery,
                    Self::Recovering
                )
        )
    }
}

/// Candidate update lifecycle and irreversible hook commit point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    /// Candidate is durable but the run gate is not yet closed.
    Candidate,
    /// Run gate is closed and old work is draining.
    Draining,
    /// Hook is executing in an isolated guest.
    HookRunning,
    /// Hook success is durable and activation must finish.
    HookCommitted,
    /// Candidate revision is active.
    Activated,
    /// Agent explicitly reported safe rollback with a nonzero exit.
    Rejected,
    /// Abnormal failure left state compatibility unknown.
    CompatibilityUnknown,
    /// Activation after committed success requires operator recovery.
    ActivationRecovery,
}

impl UpdateState {
    /// Returns whether a requested lifecycle transition honors the update exit
    /// contract.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Candidate, Self::Draining | Self::Rejected)
                | (Self::Draining, Self::HookRunning | Self::Rejected)
                | (
                    Self::HookRunning,
                    Self::HookCommitted | Self::Rejected | Self::CompatibilityUnknown
                )
                | (
                    Self::HookCommitted,
                    Self::Activated | Self::ActivationRecovery
                )
                | (Self::ActivationRecovery, Self::Activated)
        )
    }
}

/// One immutable source-repository-owned agent family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFamily {
    /// Stable family identifier.
    pub id: AgentFamilyId,
    /// Owning source repository.
    pub repository_id: RepositoryId,
    /// Stable repository-scoped exported key.
    pub agent_key: AgentKey,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Complete exact isolated-build request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    /// Durable build identity.
    pub id: BuildRequestId,
    /// Source repository.
    pub repository_id: RepositoryId,
    /// Exact source commit.
    pub source_commit: CommitSha,
    /// Source ref provenance.
    pub source_ref: GitRef,
    /// Normalized build-definition hash.
    pub build_definition_hash: ContentHash,
    /// Current lifecycle.
    pub state: BuildState,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Immutable or draft release metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Release identifier.
    pub id: ReleaseId,
    /// Source repository.
    pub repository_id: RepositoryId,
    /// Repository-selected version.
    pub version: ReleaseVersion,
    /// Exact source commit.
    pub source_commit: CommitSha,
    /// Exact source ref provenance.
    pub source_ref: GitRef,
    /// Build that produced every artifact.
    pub build_request_id: BuildRequestId,
    /// Hash of the normalized source configuration.
    pub configuration_hash: ContentHash,
    /// Complete artifact-manifest hash.
    pub manifest_hash: ContentHash,
    /// Lifecycle.
    pub state: ReleaseState,
    /// Publication time, once frozen.
    pub published_at: Option<OffsetDateTime>,
}

/// Release artifact kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Executable runtime payload.
    Executable,
    /// Immutable data file.
    File,
    /// Complete release manifest.
    Manifest,
    /// Build log stream.
    BuildLog,
}

/// Immutable artifact metadata and opaque storage identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseArtifact {
    /// Artifact identifier.
    pub id: ReleaseArtifactId,
    /// Parent release.
    pub release_id: ReleaseId,
    /// Normalized path under `/release`.
    pub path: ArtifactPath,
    /// Artifact kind.
    pub kind: ArtifactKind,
    /// Unix permission bits with file-type bits excluded.
    pub mode: u16,
    /// Content hash.
    pub content_hash: ContentHash,
    /// Byte length.
    pub size_bytes: u64,
    /// Bounded media type.
    pub media_type: String,
    /// Opaque canonical storage key.
    pub storage_key: Uuid,
}

/// Immutable release-owned runtime contract for one exported agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAgent {
    /// Export identifier.
    pub id: ReleaseAgentId,
    /// Parent release.
    pub release_id: ReleaseId,
    /// Stable compatible family.
    pub family_id: AgentFamilyId,
    /// Stable exported key.
    pub agent_key: AgentKey,
    /// Mutable human-readable display name.
    pub display_name: String,
    /// Release-relative executable path.
    pub executable: ArtifactPath,
    /// Immutable arguments.
    pub arguments: Vec<String>,
    /// Release-relative working directory.
    pub working_directory: ArtifactPath,
    /// Exact immutable OCI image reference selected for guest execution.
    pub image_reference: String,
    /// Whether one persistent volume is required per consuming instance.
    pub requires_state: bool,
    /// Normalized release-owned policy ceiling.
    pub policy_ceiling: RuntimePolicy,
    /// Typed parameter declarations.
    pub parameters: Vec<ParameterDeclaration>,
    /// Optional update hook.
    pub update_hook: Option<UpdateHook>,
}

/// Bounded compute and network policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePolicy {
    /// Virtual CPU count.
    pub vcpus: u8,
    /// Guest memory in mebibytes.
    pub memory_mib: u32,
    /// Provider-neutral network ceiling.
    pub network: NetworkAccess,
}

impl RuntimePolicy {
    /// Validates a consumer selection against a release ceiling and platform
    /// policy.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::PolicyBroadening`] when any dimension
    /// exceeds either ceiling.
    pub fn resolve(
        release: &Self,
        project: &Self,
        platform: &Self,
    ) -> Result<Self, ReleaseValueError> {
        if project.vcpus > release.vcpus
            || project.memory_mib > release.memory_mib
            || project.network.broader_than(release.network)
            || project.vcpus > platform.vcpus
            || project.memory_mib > platform.memory_mib
            || project.network.broader_than(platform.network)
        {
            return Err(ReleaseValueError::PolicyBroadening);
        }
        Ok(project.clone())
    }
}

/// Ordered provider-neutral network restriction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    /// No guest networking.
    Disabled,
    /// Only host broker connectivity.
    BrokerOnly,
    /// Constrained external egress.
    Egress,
}

impl NetworkAccess {
    const fn rank(self) -> u8 {
        match self {
            Self::Disabled => 0,
            Self::BrokerOnly => 1,
            Self::Egress => 2,
        }
    }

    const fn broader_than(self, other: Self) -> bool {
        self.rank() > other.rank()
    }
}

/// Candidate-release update-hook contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateHook {
    /// Release-relative executable.
    pub executable: ArtifactPath,
    /// Fixed arguments.
    pub arguments: Vec<String>,
    /// Wall-clock timeout in seconds.
    pub timeout_seconds: u32,
    /// Update-specific policy within the release ceiling.
    pub policy: RuntimePolicy,
}

/// Typed parameter declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterDeclaration {
    /// Stable parameter name.
    pub name: ParameterName,
    /// Parameter type and validation constraint.
    pub value_type: ParameterType,
    /// Whether consumers must explicitly or implicitly resolve a value.
    pub required: bool,
    /// Optional validated default.
    pub default: Option<ParameterValue>,
    /// Whether telemetry and UI should redact the ordinary value.
    pub sensitive: bool,
}

/// Bounded typed parameter schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParameterType {
    /// UTF-8 string with an explicit character bound.
    String {
        /// Minimum character count.
        minimum_length: u16,
        /// Maximum character count.
        maximum_length: u16,
    },
    /// Signed integer with inclusive bounds.
    Integer {
        /// Inclusive minimum.
        minimum: i64,
        /// Inclusive maximum.
        maximum: i64,
    },
    /// Boolean.
    Boolean,
    /// One of a bounded set of exact strings.
    Enum {
        /// Accepted values.
        values: Vec<String>,
    },
}

/// Typed validated parameter value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterValue {
    /// String or enum value.
    String(String),
    /// Integer value.
    Integer(i64),
    /// Boolean value.
    Boolean(bool),
}

impl ParameterType {
    /// Validates one value without coercion.
    #[must_use]
    pub fn accepts(&self, value: &ParameterValue) -> bool {
        match (self, value) {
            (
                Self::String {
                    minimum_length,
                    maximum_length,
                },
                ParameterValue::String(value),
            ) => {
                let count = value.chars().count();
                (usize::from(*minimum_length)..=usize::from(*maximum_length)).contains(&count)
            }
            (Self::Integer { minimum, maximum }, ParameterValue::Integer(value)) => {
                (*minimum..=*maximum).contains(value)
            }
            (Self::Boolean, ParameterValue::Boolean(_)) => true,
            (Self::Enum { values }, ParameterValue::String(value)) => values.contains(value),
            _ => false,
        }
    }
}

/// Canonical validated parameter document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterDocument {
    values: BTreeMap<ParameterName, ParameterValue>,
    hash: ContentHash,
}

impl ParameterDocument {
    /// Resolves provided values and defaults against the complete declaration.
    ///
    /// # Errors
    ///
    /// Returns a stable [`ParameterDiagnostic`] list for duplicate declaration
    /// names, unknown/missing parameters, or type/range mismatches.
    pub fn resolve(
        declarations: &[ParameterDeclaration],
        provided: &BTreeMap<ParameterName, ParameterValue>,
    ) -> Result<Self, Vec<ParameterDiagnostic>> {
        let mut diagnostics = Vec::new();
        let mut declarations_by_name = BTreeMap::new();
        for declaration in declarations {
            if declarations_by_name
                .insert(declaration.name.clone(), declaration)
                .is_some()
            {
                diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::DuplicateDeclaration,
                    parameter: Some(declaration.name.clone()),
                });
            }
            if declaration
                .default
                .as_ref()
                .is_some_and(|value| !declaration.value_type.accepts(value))
            {
                diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::InvalidDefault,
                    parameter: Some(declaration.name.clone()),
                });
            }
        }
        for name in provided.keys() {
            if !declarations_by_name.contains_key(name) {
                diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::UnknownParameter,
                    parameter: Some(name.clone()),
                });
            }
        }

        let mut values = BTreeMap::new();
        for (name, declaration) in declarations_by_name {
            let value = provided
                .get(&name)
                .cloned()
                .or_else(|| declaration.default.clone());
            match value {
                Some(value) if declaration.value_type.accepts(&value) => {
                    values.insert(name, value);
                }
                Some(_) => diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::InvalidValue,
                    parameter: Some(name),
                }),
                None if declaration.required => diagnostics.push(ParameterDiagnostic {
                    code: ParameterDiagnosticCode::RequiredMissing,
                    parameter: Some(name),
                }),
                None => {}
            }
        }
        if !diagnostics.is_empty() {
            diagnostics.sort_by(|left, right| {
                left.parameter
                    .cmp(&right.parameter)
                    .then_with(|| (left.code as u8).cmp(&(right.code as u8)))
            });
            return Err(diagnostics);
        }
        let hash = parameter_hash(&values);
        Ok(Self { values, hash })
    }

    /// Returns the sorted validated values.
    #[must_use]
    pub const fn values(&self) -> &BTreeMap<ParameterName, ParameterValue> {
        &self.values
    }

    /// Returns the deterministic canonical parameter hash.
    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }
}

fn parameter_hash(values: &BTreeMap<ParameterName, ParameterValue>) -> ContentHash {
    let mut digest = Sha256::new();
    for (name, value) in values {
        update_field(&mut digest, name.as_str().as_bytes());
        match value {
            ParameterValue::String(value) => {
                update_field(&mut digest, b"string");
                update_field(&mut digest, value.as_bytes());
            }
            ParameterValue::Integer(value) => {
                update_field(&mut digest, b"integer");
                update_field(&mut digest, &value.to_be_bytes());
            }
            ParameterValue::Boolean(value) => {
                update_field(&mut digest, b"boolean");
                update_field(&mut digest, &[u8::from(*value)]);
            }
        }
    }
    ContentHash(digest.finalize().into())
}

/// Stable parameter compatibility diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterDiagnosticCode {
    /// Release declared the same name more than once.
    DuplicateDeclaration,
    /// A release-owned default violates its own schema.
    InvalidDefault,
    /// Consumer supplied a name the release did not declare.
    UnknownParameter,
    /// Required parameter has no supplied or default value.
    RequiredMissing,
    /// Supplied value has the wrong type or is out of bounds.
    InvalidValue,
}

/// Structured stable parameter diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterDiagnostic {
    /// Stable code.
    pub code: ParameterDiagnosticCode,
    /// Relevant parameter name.
    pub parameter: Option<ParameterName>,
}

/// Project-owned instance aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstance {
    /// Stable product-level identity.
    pub id: AgentInstanceId,
    /// Consuming project.
    pub project_id: ProjectId,
    /// Project-scoped name.
    pub name: InstanceName,
    /// Compatible source family.
    pub family_id: AgentFamilyId,
    /// Current lifecycle.
    pub state: InstanceState,
    /// Exact active immutable revision.
    pub active_revision_id: AgentInstanceRevisionId,
    /// Optional one-per-instance state volume.
    pub state_volume_id: Option<VolumeId>,
    /// Optimistic aggregate version.
    pub version: u64,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Immutable completely resolved instance revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstanceRevision {
    /// Revision identifier.
    pub id: AgentInstanceRevisionId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Exact immutable release export.
    pub release_agent_id: ReleaseAgentId,
    /// Validated parameter document.
    pub parameters: ParameterDocument,
    /// Opaque secret binding UUIDs. Plaintext and ciphertext cannot appear.
    pub secret_binding_ids: Vec<Uuid>,
    /// Fully resolved effective runtime policy.
    pub effective_policy: RuntimePolicy,
    /// Platform policy version used for validation.
    pub platform_policy_version: String,
    /// Whether this revision can currently run.
    pub runnable: bool,
    /// Stable non-sensitive invalidity diagnostics.
    pub diagnostics: Vec<RevisionDiagnostic>,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Stable revision validation diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionDiagnostic {
    /// Stable code.
    pub code: String,
    /// Non-sensitive field key.
    pub field: Option<String>,
}

/// Attachment trigger policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerPolicy {
    /// Accepted pushes matching the ref selector create requests.
    Push,
    /// Only explicit authorized commands create requests.
    Manual,
    /// Both push and explicit commands are accepted.
    PushAndManual,
}

/// One repository/ref attachment for a reusable instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAttachment {
    /// Attachment identity.
    pub id: AgentAttachmentId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Target repository in the consuming project.
    pub repository_id: RepositoryId,
    /// Bounded ref selection.
    pub ref_selector: RefSelector,
    /// Trigger policy owned by this attachment.
    pub trigger_policy: TriggerPolicy,
    /// Whether new triggers are accepted.
    pub enabled: bool,
    /// Tombstone time. Historical runs keep resolving this row.
    pub removed_at: Option<OffsetDateTime>,
}

/// Exact candidate update with compare-and-swap provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUpdate {
    /// Stable update identity delivered to hooks.
    pub id: AgentUpdateId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Revision expected to remain active until the update commits.
    pub expected_current_revision_id: AgentInstanceRevisionId,
    /// Fully validated candidate.
    pub candidate_revision_id: AgentInstanceRevisionId,
    /// Durable lifecycle including irreversible hook success.
    pub state: UpdateState,
    /// Optional exact update-hook run.
    pub hook_run_id: Option<RunId>,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Trigger accepted while an instance run gate is closed. It deliberately has
/// no instance revision field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredTrigger {
    /// Stable trigger identity.
    pub id: DeferredTriggerId,
    /// Matching attachment.
    pub attachment_id: AgentAttachmentId,
    /// Exact target repository.
    pub repository_id: RepositoryId,
    /// Exact target ref.
    pub target_ref: GitRef,
    /// Exact target commit.
    pub target_commit: CommitSha,
    /// Source receive or command UUID.
    pub source_id: Uuid,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Protected exact provenance for one normal run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactRunProvenance {
    /// Run identity.
    pub run_id: RunId,
    /// Product-level agent.
    pub instance_id: AgentInstanceId,
    /// Exact immutable revision.
    pub instance_revision_id: AgentInstanceRevisionId,
    /// Exact source release.
    pub release_id: ReleaseId,
    /// Exact release export.
    pub release_agent_id: ReleaseAgentId,
    /// Exact attachment.
    pub attachment_id: AgentAttachmentId,
    /// Exact target repository.
    pub target_repository_id: RepositoryId,
    /// Exact target ref.
    pub target_ref: GitRef,
    /// Exact target commit.
    pub target_commit: CommitSha,
    /// Immutable parameter hash.
    pub parameter_hash: ContentHash,
    /// Platform policy version verified at dispatch.
    pub platform_policy_version: String,
}

/// SHA-256 content or normalized-document hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hashes exact bytes.
    #[must_use]
    pub fn digest(value: &[u8]) -> Self {
        Self(Sha256::digest(value).into())
    }

    /// Wraps an already computed SHA-256 digest.
    #[must_use]
    pub const fn from_digest(value: [u8; 32]) -> Self {
        Self(value)
    }

    /// Returns raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Deterministic identity for build, publication, revision, attachment,
/// update, and run commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReleaseCommandKey([u8; 32]);

impl ReleaseCommandKey {
    /// Derives an unambiguous operation-scoped identity.
    #[must_use]
    pub fn derive(operation: &str, fields: &[&[u8]]) -> Self {
        let mut digest = Sha256::new();
        update_field(&mut digest, operation.as_bytes());
        for field in fields {
            update_field(&mut digest, field);
        }
        Self(digest.finalize().into())
    }

    /// Returns raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn update_field(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_be_bytes());
    digest.update(value);
}

/// Validates that an update remains in the exact source family.
///
/// # Errors
///
/// Returns [`ReleaseValueError::IncompatibleAgentFamily`] when a same-named
/// export from another repository/family is supplied.
pub fn validate_update_family(
    instance_family_id: AgentFamilyId,
    candidate_family_id: AgentFamilyId,
) -> Result<(), ReleaseValueError> {
    if instance_family_id.as_uuid() == candidate_family_id.as_uuid() {
        Ok(())
    } else {
        Err(ReleaseValueError::IncompatibleAgentFamily)
    }
}

/// Validates that an attachment stays inside the consuming project.
///
/// # Errors
///
/// Returns [`ReleaseValueError::CrossProjectAttachment`] for a mismatch.
pub fn validate_attachment_project(
    instance_project_id: ProjectId,
    repository_project_id: ProjectId,
) -> Result<(), ReleaseValueError> {
    if instance_project_id.as_uuid() == repository_project_id.as_uuid() {
        Ok(())
    } else {
        Err(ReleaseValueError::CrossProjectAttachment)
    }
}

/// Release-domain validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReleaseValueError {
    /// A stable key is malformed.
    #[error("{kind} must be a bounded lowercase identifier")]
    InvalidKey {
        /// Value-object kind, never rejected user input.
        kind: &'static str,
    },
    /// Release version is malformed.
    #[error("release version is invalid")]
    InvalidVersion,
    /// Artifact path is unsafe.
    #[error("artifact path must be relative, traversal-free, and outside .git")]
    InvalidArtifactPath,
    /// Ref selector is malformed.
    #[error("ref selector must be an exact fully-qualified ref or terminal prefix")]
    InvalidRefSelector,
    /// Project attempted to broaden release or platform policy.
    #[error("project runtime selection exceeds a release or platform ceiling")]
    PolicyBroadening,
    /// Candidate belongs to another source repository/family.
    #[error("candidate release agent belongs to a different agent family")]
    IncompatibleAgentFamily,
    /// Attachment repository belongs to another project.
    #[error("agent attachment repository belongs to a different project")]
    CrossProjectAttachment,
}

#[cfg(test)]
mod tests {
    use super::{
        AgentFamilyId, AgentKey, ArtifactPath, BuildState, ContentHash, InstanceState,
        NetworkAccess, ParameterDeclaration, ParameterDiagnosticCode, ParameterDocument,
        ParameterName, ParameterType, ParameterValue, RefSelector, ReleaseCommandKey, ReleaseState,
        ReleaseVersion, RuntimePolicy, UpdateState, validate_attachment_project,
        validate_update_family,
    };
    use forge_domain::{GitRef, ProjectId};
    use std::collections::BTreeMap;

    #[test]
    fn identifiers_and_bounded_values_round_trip() {
        let id = super::ReleaseId::new();
        let serialized = serde_json::to_string(&id).expect("identifier should serialize");
        assert_eq!(
            serde_json::from_str::<super::ReleaseId>(&serialized)
                .expect("identifier should deserialize"),
            id
        );
        assert_eq!(
            id.to_string()
                .parse::<super::ReleaseId>()
                .expect("identifier should parse"),
            id
        );
        assert!("invalid".parse::<super::ReleaseId>().is_err());
        assert!(ReleaseVersion::parse("v1.2.3+linux").is_ok());
        assert!(ReleaseVersion::parse("../escape").is_err());
        assert!(AgentKey::parse("reviewer-v2").is_ok());
        assert!(AgentKey::parse("Reviewer").is_err());
        assert!(ArtifactPath::parse("bin/reviewer").is_ok());
        assert!(ArtifactPath::parse("../bin/reviewer").is_err());
        assert!(ArtifactPath::parse(".git/config").is_err());
    }

    #[test]
    fn ref_selectors_match_exactly_or_below_prefix() {
        let exact = RefSelector::parse("refs/heads/main").expect("exact selector should parse");
        let prefix =
            RefSelector::parse("refs/heads/release/*").expect("prefix selector should parse");
        let main = GitRef::parse("refs/heads/main").expect("ref should parse");
        let release = GitRef::parse("refs/heads/release/v1").expect("ref should parse");
        assert!(exact.matches(&main));
        assert!(!exact.matches(&release));
        assert!(prefix.matches(&release));
        assert!(!prefix.matches(&main));
        assert!(RefSelector::parse("../../main").is_err());
    }

    #[test]
    fn lifecycle_transitions_enforce_commit_points_and_run_gate() {
        assert!(BuildState::Queued.can_transition_to(BuildState::Running));
        assert!(!BuildState::Succeeded.can_transition_to(BuildState::Running));
        assert!(ReleaseState::Draft.can_transition_to(ReleaseState::Published));
        assert!(!ReleaseState::Published.can_transition_to(ReleaseState::Draft));

        assert!(InstanceState::Active.run_gate_open());
        assert!(!InstanceState::Updating.run_gate_open());
        assert!(InstanceState::Updating.can_transition_to(InstanceState::PausedUnknownState));
        assert!(UpdateState::HookRunning.can_transition_to(UpdateState::HookCommitted));
        assert!(UpdateState::HookCommitted.can_transition_to(UpdateState::ActivationRecovery));
        assert!(!UpdateState::HookCommitted.can_transition_to(UpdateState::Rejected));
    }

    #[test]
    fn family_and_project_boundaries_use_stable_ids_not_names() {
        let family = AgentFamilyId::new();
        assert!(validate_update_family(family, family).is_ok());
        assert!(validate_update_family(family, AgentFamilyId::new()).is_err());

        let project = ProjectId::new();
        assert!(validate_attachment_project(project, project).is_ok());
        assert!(validate_attachment_project(project, ProjectId::new()).is_err());
    }

    #[test]
    fn effective_policy_can_only_restrict() {
        let release = RuntimePolicy {
            vcpus: 8,
            memory_mib: 8192,
            network: NetworkAccess::Egress,
        };
        let platform = RuntimePolicy {
            vcpus: 16,
            memory_mib: 16_384,
            network: NetworkAccess::BrokerOnly,
        };
        let selected = RuntimePolicy {
            vcpus: 4,
            memory_mib: 4096,
            network: NetworkAccess::BrokerOnly,
        };
        assert_eq!(
            RuntimePolicy::resolve(&release, &selected, &platform)
                .expect("selection should fit both ceilings"),
            selected
        );
        let too_broad = RuntimePolicy {
            network: NetworkAccess::Egress,
            ..selected
        };
        assert!(RuntimePolicy::resolve(&release, &too_broad, &platform).is_err());
    }

    #[test]
    fn typed_parameters_are_complete_and_deterministic() {
        let count = ParameterName::parse("count").expect("name should validate");
        let mode = ParameterName::parse("mode").expect("name should validate");
        let declarations = vec![
            ParameterDeclaration {
                name: count.clone(),
                value_type: ParameterType::Integer {
                    minimum: 1,
                    maximum: 10,
                },
                required: true,
                default: None,
                sensitive: false,
            },
            ParameterDeclaration {
                name: mode.clone(),
                value_type: ParameterType::Enum {
                    values: vec![String::from("fast"), String::from("safe")],
                },
                required: false,
                default: Some(ParameterValue::String(String::from("safe"))),
                sensitive: false,
            },
        ];
        let values = BTreeMap::from([(count.clone(), ParameterValue::Integer(3))]);
        let first =
            ParameterDocument::resolve(&declarations, &values).expect("parameters should resolve");
        let repeated_values = BTreeMap::from([(count, ParameterValue::Integer(3))]);
        let second = ParameterDocument::resolve(&declarations, &repeated_values)
            .expect("parameters should resolve again");
        assert_eq!(first.hash(), second.hash());
        assert_eq!(
            first.values().get(&mode),
            Some(&ParameterValue::String(String::from("safe")))
        );

        let diagnostics = ParameterDocument::resolve(&declarations, &BTreeMap::new())
            .expect_err("required value should be diagnosed");
        assert_eq!(
            diagnostics[0].code,
            ParameterDiagnosticCode::RequiredMissing
        );
    }

    #[test]
    fn release_update_parameter_diagnostics_are_stable_for_schema_changes() {
        let legacy = ParameterName::parse("legacy").expect("name should validate");
        let replacement = ParameterName::parse("replacement").expect("name should validate");
        let provided = BTreeMap::from([(
            legacy.clone(),
            ParameterValue::String(String::from("value")),
        )]);

        let removed = ParameterDocument::resolve(&[], &provided)
            .expect_err("removed parameter should be rejected");
        assert_eq!(
            removed,
            vec![super::ParameterDiagnostic {
                code: ParameterDiagnosticCode::UnknownParameter,
                parameter: Some(legacy.clone()),
            }]
        );

        let renamed_declarations = vec![ParameterDeclaration {
            name: replacement.clone(),
            value_type: ParameterType::String {
                minimum_length: 1,
                maximum_length: 32,
            },
            required: true,
            default: None,
            sensitive: false,
        }];
        let renamed = ParameterDocument::resolve(&renamed_declarations, &provided)
            .expect_err("renamed parameter should produce complete diagnostics");
        assert_eq!(
            renamed,
            vec![
                super::ParameterDiagnostic {
                    code: ParameterDiagnosticCode::UnknownParameter,
                    parameter: Some(legacy.clone()),
                },
                super::ParameterDiagnostic {
                    code: ParameterDiagnosticCode::RequiredMissing,
                    parameter: Some(replacement.clone()),
                },
            ]
        );

        let newly_required = ParameterDocument::resolve(&renamed_declarations, &BTreeMap::new())
            .expect_err("newly required parameter should be rejected");
        assert_eq!(
            newly_required,
            vec![super::ParameterDiagnostic {
                code: ParameterDiagnosticCode::RequiredMissing,
                parameter: Some(replacement),
            }]
        );

        let type_changed = ParameterDocument::resolve(
            &[ParameterDeclaration {
                name: legacy.clone(),
                value_type: ParameterType::Boolean,
                required: true,
                default: None,
                sensitive: false,
            }],
            &provided,
        )
        .expect_err("type-changed value should be rejected without coercion");
        assert_eq!(
            type_changed,
            vec![super::ParameterDiagnostic {
                code: ParameterDiagnosticCode::InvalidValue,
                parameter: Some(legacy),
            }]
        );
    }

    #[test]
    fn deterministic_hashes_and_command_keys_are_unambiguous() {
        assert_eq!(
            ContentHash::digest(b"config"),
            ContentHash::digest(b"config")
        );
        assert_ne!(
            ContentHash::digest(b"config"),
            ContentHash::digest(b"artifact")
        );

        let first = ReleaseCommandKey::derive("build", &[b"ab", b"c"]);
        assert_eq!(first, ReleaseCommandKey::derive("build", &[b"ab", b"c"]));
        assert_ne!(first, ReleaseCommandKey::derive("build", &[b"a", b"bc"]));
        assert_ne!(first, ReleaseCommandKey::derive("release", &[b"ab", b"c"]));
    }
}

//! Strict, provider-neutral Git capability grammar.
//!
//! The types in this crate describe authority; they do not authenticate a
//! credential or inspect a repository. Transport adapters must obtain trusted
//! repository state and use it to construct the transitions checked here.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt, str::FromStr};
use uuid::Uuid;

/// Current version of the normalized Git capability grammar.
pub const GRAMMAR_VERSION: u16 = 1;

const MAX_REF_GLOB_BYTES: usize = 512;
const MAX_PATH_GLOB_BYTES: usize = 1_024;
const MAX_GLOBS: usize = 256;
const MAX_REQUEST_BYTES: u64 = 16 * 1_024 * 1_024;
const MAX_PACK_BYTES: u64 = 1_024 * 1_024 * 1_024;
const MAX_OBJECTS: u32 = 1_000_000;
const MAX_REF_UPDATES: u16 = 256;

/// A canonical opaque repository identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryId(Uuid);

impl RepositoryId {
    /// Creates an identifier from a UUID.
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    /// Returns the underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl FromStr for RepositoryId {
    type Err = GitCapabilityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parsed = Uuid::parse_str(value)
            .map_err(|_| GitCapabilityError::NonCanonicalRepositoryId(value.to_owned()))?;
        if parsed.hyphenated().to_string() != value {
            return Err(GitCapabilityError::NonCanonicalRepositoryId(
                value.to_owned(),
            ));
        }
        Ok(Self(parsed))
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(formatter)
    }
}

/// A smart-HTTP Git operation authorized by a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperation {
    /// Discover visible refs through upload-pack advertisement.
    Discover,
    /// Fetch objects through upload-pack.
    Fetch,
    /// Propose atomic ref changes through receive-pack.
    Receive,
}

/// A strict, anchored glob over fully qualified Git refs.
///
/// Matching is case-sensitive over Unicode scalar values. Unicode
/// normalization is deliberately not performed, so canonically equivalent
/// spellings remain distinct. `*` matches within one slash-delimited segment;
/// a segment equal to `**` matches zero or more complete segments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RefGlob(String);

impl RefGlob {
    /// Parses a bounded glob and rejects a whole-namespace pattern.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, unbounded, ambiguous, or implicitly
    /// whole-namespace patterns.
    pub fn parse(value: impl Into<String>) -> Result<Self, GitCapabilityError> {
        Self::parse_inner(value.into(), false)
    }

    /// Parses a bounded glob while explicitly permitting a whole namespace.
    ///
    /// Callers should expose this separately from ordinary narrow-scope input.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern is malformed, unbounded, or
    /// ambiguous.
    pub fn parse_explicitly_broad(value: impl Into<String>) -> Result<Self, GitCapabilityError> {
        Self::parse_inner(value.into(), true)
    }

    fn parse_inner(value: String, broad: bool) -> Result<Self, GitCapabilityError> {
        validate_ref_glob(&value)?;
        if !broad && is_broad_ref_glob(&value) {
            return Err(GitCapabilityError::BroadGlobRequiresExplicitOptIn(value));
        }
        Ok(Self(value))
    }

    /// Returns the normalized glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether this anchored glob matches a validated ref.
    #[must_use]
    pub fn is_match(&self, reference: &str) -> bool {
        validate_concrete_ref(reference).is_ok() && glob_matches(&self.0, reference)
    }
}

/// A strict, repository-relative glob over changed Git paths.
///
/// Paths and patterns use `/` separators. Matching has the same exact Unicode,
/// case, `*`, and `**` semantics as [`RefGlob`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ChangedPathGlob(String);

impl ChangedPathGlob {
    /// Parses a bounded path glob and rejects a repository-wide pattern.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, unbounded, ambiguous, or implicitly
    /// repository-wide patterns.
    pub fn parse(value: impl Into<String>) -> Result<Self, GitCapabilityError> {
        Self::parse_inner(value.into(), false)
    }

    /// Parses a bounded path glob while explicitly permitting repository-wide
    /// matching.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern is malformed, unbounded, or
    /// ambiguous.
    pub fn parse_explicitly_broad(value: impl Into<String>) -> Result<Self, GitCapabilityError> {
        Self::parse_inner(value.into(), true)
    }

    fn parse_inner(value: String, broad: bool) -> Result<Self, GitCapabilityError> {
        validate_path_glob(&value)?;
        if !broad && matches!(value.as_str(), "*" | "**") {
            return Err(GitCapabilityError::BroadGlobRequiresExplicitOptIn(value));
        }
        Ok(Self(value))
    }

    /// Returns the normalized glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether this anchored glob matches a validated changed path.
    #[must_use]
    pub fn is_match(&self, path: &str) -> bool {
        validate_concrete_path(path).is_ok() && glob_matches(&self.0, path)
    }
}

/// Policy for updates to an existing branch ref.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchUpdatePolicy {
    /// Permit only updates proven to be fast-forward.
    FastForwardOnly,
    /// Permit both fast-forward and non-fast-forward updates.
    AllowForce,
}

/// Whether one explicit ref mutation is permitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefMutationPermission {
    /// Reject the mutation.
    Deny,
    /// Permit the mutation when all other scope checks pass.
    Allow,
}

impl RefMutationPermission {
    const fn allows(self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Creation, update, and deletion policy for branch refs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRefPolicy {
    /// Rule for existing branch updates.
    pub updates: BranchUpdatePolicy,
    /// Rule for branch creation.
    pub create: RefMutationPermission,
    /// Rule for branch deletion.
    pub delete: RefMutationPermission,
}

impl Default for BranchRefPolicy {
    fn default() -> Self {
        Self {
            updates: BranchUpdatePolicy::FastForwardOnly,
            create: RefMutationPermission::Deny,
            delete: RefMutationPermission::Deny,
        }
    }
}

/// Creation, update, and deletion policy for non-branch refs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefNamespacePolicy {
    /// Rule for ref creation.
    pub create: RefMutationPermission,
    /// Rule for changing an existing ref.
    pub update: RefMutationPermission,
    /// Rule for ref deletion.
    pub delete: RefMutationPermission,
}

impl Default for RefNamespacePolicy {
    fn default() -> Self {
        Self {
            create: RefMutationPermission::Deny,
            update: RefMutationPermission::Deny,
            delete: RefMutationPermission::Deny,
        }
    }
}

/// Explicit creation, update, and deletion rules for matched ref namespaces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefUpdatePolicy {
    /// Branch-ref policy.
    pub branches: BranchRefPolicy,
    /// Tag-ref policy.
    pub tags: RefNamespacePolicy,
    /// Policy below other explicit `refs/<namespace>/` namespaces.
    pub other: RefNamespacePolicy,
}

impl RefUpdatePolicy {
    fn permits(self, kind: RefKind, transition: RefTransition) -> bool {
        match (kind, transition) {
            (RefKind::Branch, RefTransition::Create) => self.branches.create.allows(),
            (RefKind::Branch, RefTransition::Delete) => self.branches.delete.allows(),
            (RefKind::Branch, RefTransition::Update { fast_forward: true }) => true,
            (
                RefKind::Branch,
                RefTransition::Update {
                    fast_forward: false,
                },
            ) => self.branches.updates == BranchUpdatePolicy::AllowForce,
            (RefKind::Tag, RefTransition::Create) => self.tags.create.allows(),
            (RefKind::Tag, RefTransition::Delete) => self.tags.delete.allows(),
            (RefKind::Tag, RefTransition::Update { .. }) => self.tags.update.allows(),
            (RefKind::Other, RefTransition::Create) => self.other.create.allows(),
            (RefKind::Other, RefTransition::Delete) => self.other.delete.allows(),
            (RefKind::Other, RefTransition::Update { .. }) => self.other.update.allows(),
        }
    }
}

/// Bounded smart-HTTP request and object-transfer limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferLimits {
    request_bytes: u64,
    pack_bytes: u64,
    object_count: u32,
    ref_updates: u16,
}

impl TransferLimits {
    /// Creates non-zero limits within the grammar's hard ceilings.
    ///
    /// # Errors
    ///
    /// Returns an error when any limit is zero or exceeds its hard ceiling.
    pub const fn new(
        request_bytes: u64,
        pack_bytes: u64,
        object_count: u32,
        ref_updates: u16,
    ) -> Result<Self, GitCapabilityError> {
        if request_bytes == 0
            || request_bytes > MAX_REQUEST_BYTES
            || pack_bytes == 0
            || pack_bytes > MAX_PACK_BYTES
            || object_count == 0
            || object_count > MAX_OBJECTS
            || ref_updates == 0
            || ref_updates > MAX_REF_UPDATES
        {
            return Err(GitCapabilityError::InvalidTransferLimits);
        }
        Ok(Self {
            request_bytes,
            pack_bytes,
            object_count,
            ref_updates,
        })
    }

    /// Maximum encoded request bytes.
    #[must_use]
    pub const fn request_bytes(self) -> u64 {
        self.request_bytes
    }

    /// Maximum accepted pack bytes.
    #[must_use]
    pub const fn pack_bytes(self) -> u64 {
        self.pack_bytes
    }

    /// Maximum accepted object count.
    #[must_use]
    pub const fn object_count(self) -> u32 {
        self.object_count
    }

    /// Maximum atomic ref updates in one receive.
    #[must_use]
    pub const fn ref_updates(self) -> u16 {
        self.ref_updates
    }
}

/// Unnormalized input used to construct a capability scope.
#[derive(Debug, Clone)]
pub struct GitCapabilityScopeInput {
    /// Exact repository binding.
    pub repository_id: RepositoryId,
    /// Authorized operations; construction sorts and deduplicates them.
    pub operations: Vec<GitOperation>,
    /// Authorized ref globs; construction sorts and deduplicates them.
    pub ref_globs: Vec<RefGlob>,
    /// Authorized changed-path globs for receive operations.
    pub changed_path_globs: Vec<ChangedPathGlob>,
    /// Ref transition policy.
    pub update_policy: RefUpdatePolicy,
    /// Exclusive expiry as whole Unix seconds.
    pub expires_at_unix_seconds: i64,
    /// Transfer ceilings.
    pub transfer_limits: TransferLimits,
}

/// Unnormalized release- or instance-owned Git authority rules.
///
/// Unlike [`GitCapabilityScopeInput`], these rules do not contain an exact
/// repository or expiry. A release uses them as a maximum ceiling and an
/// instance revision binds an equal or narrower value to one repository.
#[derive(Debug, Clone)]
pub struct GitCapabilityCeilingInput {
    /// Authorized operations; construction sorts and deduplicates them.
    pub operations: Vec<GitOperation>,
    /// Authorized ref globs; construction sorts and deduplicates them.
    pub ref_globs: Vec<RefGlob>,
    /// Authorized changed-path globs for receive operations.
    pub changed_path_globs: Vec<ChangedPathGlob>,
    /// Ref transition policy.
    pub update_policy: RefUpdatePolicy,
    /// Transfer ceilings.
    pub transfer_limits: TransferLimits,
    /// Whether dispatch must bind the triggering commit as the exact old
    /// commit accepted by receive.
    pub exact_parent_required: bool,
}

/// A validated, normalized release Git authority ceiling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitCapabilityCeiling {
    version: u16,
    operations: Vec<GitOperation>,
    ref_globs: Vec<RefGlob>,
    changed_path_globs: Vec<ChangedPathGlob>,
    update_policy: RefUpdatePolicy,
    transfer_limits: TransferLimits,
    exact_parent_required: bool,
}

impl GitCapabilityCeiling {
    /// Validates and normalizes one repository-independent Git ceiling.
    ///
    /// # Errors
    ///
    /// Returns an error for empty or oversized collections or receive fields
    /// that conflict with the operation set.
    pub fn new(input: GitCapabilityCeilingInput) -> Result<Self, GitCapabilityError> {
        let operations = normalize_bounded(input.operations, "operations")?;
        let ref_globs = normalize_bounded(input.ref_globs, "ref_globs")?;
        let changed_path_globs = normalize_optional_bounded(input.changed_path_globs)?;
        let receives = operations.contains(&GitOperation::Receive);
        if receives == changed_path_globs.is_empty() {
            return Err(GitCapabilityError::ConflictingScope(
                "receive authority and changed-path globs must be declared together",
            ));
        }
        if !receives && input.update_policy != RefUpdatePolicy::default() {
            return Err(GitCapabilityError::ConflictingScope(
                "ref update policy requires receive authority",
            ));
        }
        if input.exact_parent_required && !receives {
            return Err(GitCapabilityError::ConflictingScope(
                "an exact parent requires receive authority",
            ));
        }
        Ok(Self {
            version: GRAMMAR_VERSION,
            operations,
            ref_globs,
            changed_path_globs,
            update_policy: input.update_policy,
            transfer_limits: input.transfer_limits,
            exact_parent_required: input.exact_parent_required,
        })
    }

    /// Returns the grammar version included in the normalized form.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns normalized Git transport operations.
    #[must_use]
    pub fn operations(&self) -> &[GitOperation] {
        &self.operations
    }

    /// Returns normalized visible/writable ref globs.
    #[must_use]
    pub fn ref_globs(&self) -> &[RefGlob] {
        &self.ref_globs
    }

    /// Returns normalized receive changed-path globs.
    #[must_use]
    pub fn changed_path_globs(&self) -> &[ChangedPathGlob] {
        &self.changed_path_globs
    }

    /// Returns the normalized ref transition policy.
    #[must_use]
    pub const fn update_policy(&self) -> RefUpdatePolicy {
        self.update_policy
    }

    /// Returns bounded transfer limits.
    #[must_use]
    pub const fn transfer_limits(&self) -> TransferLimits {
        self.transfer_limits
    }

    /// Returns whether dispatch must snapshot an exact old commit.
    #[must_use]
    pub const fn exact_parent_required(&self) -> bool {
        self.exact_parent_required
    }

    /// Returns whether this value grants no authority beyond `ceiling`.
    ///
    /// Glob attenuation deliberately permits only removing complete declared
    /// patterns. This conservative rule is exact and avoids treating pattern
    /// text as a concrete ref or attempting an unsound glob-containment test.
    #[must_use]
    pub fn is_attenuation_of(&self, ceiling: &Self) -> bool {
        self.operations
            .iter()
            .all(|value| ceiling.operations.contains(value))
            && self
                .ref_globs
                .iter()
                .all(|value| ceiling.ref_globs.contains(value))
            && self
                .changed_path_globs
                .iter()
                .all(|value| ceiling.changed_path_globs.contains(value))
            && update_policy_is_attenuation(self.update_policy, ceiling.update_policy)
            && limits_are_attenuation(self.transfer_limits, ceiling.transfer_limits)
            && (!ceiling.exact_parent_required || self.exact_parent_required)
    }

    /// Returns canonical JSON bytes used for persistence and hashing.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn canonical_json(&self) -> Result<Vec<u8>, GitCapabilityError> {
        serde_json::to_vec(self).map_err(GitCapabilityError::CanonicalSerialization)
    }

    /// Returns the versioned normalized ceiling hash.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn normalized_hash(&self) -> Result<GitCapabilityHash, GitCapabilityError> {
        Ok(git_hash(&self.canonical_json()?))
    }
}

/// One exact repository bound to normalized Git authority rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundGitCapability {
    repository_id: RepositoryId,
    authority: GitCapabilityCeiling,
}

impl BoundGitCapability {
    /// Binds an equal or narrower authority value to one exact repository.
    ///
    /// # Errors
    ///
    /// Returns an error if `authority` broadens the release ceiling.
    pub fn new(
        repository_id: RepositoryId,
        authority: GitCapabilityCeiling,
        release_ceiling: &GitCapabilityCeiling,
    ) -> Result<Self, GitCapabilityError> {
        if !authority.is_attenuation_of(release_ceiling) {
            return Err(GitCapabilityError::ScopeBroadening);
        }
        Ok(Self {
            repository_id,
            authority,
        })
    }

    /// Returns the exact repository binding.
    #[must_use]
    pub const fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Returns the immutable attenuated authority rules.
    #[must_use]
    pub const fn authority(&self) -> &GitCapabilityCeiling {
        &self.authority
    }

    /// Returns the hash of the repository identity and normalized rules.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn normalized_hash(&self) -> Result<GitCapabilityHash, GitCapabilityError> {
        let bytes = serde_json::to_vec(self).map_err(GitCapabilityError::CanonicalSerialization)?;
        Ok(git_hash(&bytes))
    }
}

/// A validated and deterministically normalized Git capability scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitCapabilityScope {
    version: u16,
    repository_id: RepositoryId,
    operations: Vec<GitOperation>,
    ref_globs: Vec<RefGlob>,
    changed_path_globs: Vec<ChangedPathGlob>,
    update_policy: RefUpdatePolicy,
    expires_at_unix_seconds: i64,
    transfer_limits: TransferLimits,
}

impl GitCapabilityScope {
    /// Validates and normalizes one scope.
    ///
    /// # Errors
    ///
    /// Returns an error for empty or oversized collections, receive fields
    /// that conflict with the operation set, or an invalid expiry.
    pub fn new(input: GitCapabilityScopeInput) -> Result<Self, GitCapabilityError> {
        let operations = normalize_bounded(input.operations, "operations")?;
        let ref_globs = normalize_bounded(input.ref_globs, "ref_globs")?;
        let changed_path_globs = normalize_optional_bounded(input.changed_path_globs)?;
        let receives = operations.contains(&GitOperation::Receive);

        if receives == changed_path_globs.is_empty() {
            return Err(GitCapabilityError::ConflictingScope(
                "receive authority and changed-path globs must be declared together",
            ));
        }
        if !receives && input.update_policy != RefUpdatePolicy::default() {
            return Err(GitCapabilityError::ConflictingScope(
                "ref update policy requires receive authority",
            ));
        }
        if input.expires_at_unix_seconds <= 0 {
            return Err(GitCapabilityError::InvalidExpiry);
        }

        Ok(Self {
            version: GRAMMAR_VERSION,
            repository_id: input.repository_id,
            operations,
            ref_globs,
            changed_path_globs,
            update_policy: input.update_policy,
            expires_at_unix_seconds: input.expires_at_unix_seconds,
            transfer_limits: input.transfer_limits,
        })
    }

    /// Returns the grammar version included in the normalized form.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the exact repository binding.
    #[must_use]
    pub const fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Returns normalized operations.
    #[must_use]
    pub fn operations(&self) -> &[GitOperation] {
        &self.operations
    }

    /// Returns normalized ref globs.
    #[must_use]
    pub fn ref_globs(&self) -> &[RefGlob] {
        &self.ref_globs
    }

    /// Returns normalized changed-path globs.
    #[must_use]
    pub fn changed_path_globs(&self) -> &[ChangedPathGlob] {
        &self.changed_path_globs
    }

    /// Returns the exclusive expiry as whole Unix seconds.
    #[must_use]
    pub const fn expires_at_unix_seconds(&self) -> i64 {
        self.expires_at_unix_seconds
    }

    /// Returns the normalized ref transition policy.
    #[must_use]
    pub const fn update_policy(&self) -> RefUpdatePolicy {
        self.update_policy
    }

    /// Returns the bounded transfer limits.
    #[must_use]
    pub const fn transfer_limits(&self) -> TransferLimits {
        self.transfer_limits
    }

    /// Returns whether the scope is active at the supplied whole Unix second.
    #[must_use]
    pub const fn is_active_at(&self, unix_seconds: i64) -> bool {
        unix_seconds < self.expires_at_unix_seconds
    }

    /// Returns whether an operation and ref are in scope.
    #[must_use]
    pub fn allows(&self, operation: GitOperation, reference: &str) -> bool {
        self.operations.contains(&operation)
            && self.ref_globs.iter().any(|glob| glob.is_match(reference))
    }

    /// Checks a trusted receive transition and its complete changed-path set.
    ///
    /// For a rename, both old and new paths must match. For a merge, callers
    /// must supply the union of changes against every parent. For a newly
    /// created branch, callers must supply the full diff from the empty tree.
    /// An empty changed-path set is accepted only for a ref update whose tree
    /// is unchanged.
    #[must_use]
    pub fn allows_receive(&self, update: &ReceiveUpdate<'_>) -> bool {
        self.allows(GitOperation::Receive, update.reference)
            && self
                .update_policy
                .permits(RefKind::of(update.reference), update.transition)
            && update
                .changed_paths
                .iter()
                .all(|change| self.allows_path_change(change))
    }

    /// Returns canonical JSON bytes used for persistence and hashing.
    ///
    /// Struct field order and normalized vector order are part of grammar
    /// version 1.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn canonical_json(&self) -> Result<Vec<u8>, GitCapabilityError> {
        serde_json::to_vec(self).map_err(GitCapabilityError::CanonicalSerialization)
    }

    /// Returns the SHA-256 digest of [`Self::canonical_json`].
    ///
    /// # Errors
    ///
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn normalized_hash(&self) -> Result<GitCapabilityHash, GitCapabilityError> {
        Ok(git_hash(&self.canonical_json()?))
    }

    fn allows_path_change(&self, change: &PathChange<'_>) -> bool {
        change.paths().into_iter().flatten().all(|path| {
            self.changed_path_globs
                .iter()
                .any(|glob| glob.is_match(path))
        })
    }
}

/// A trusted ref transition determined from repository state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefTransition {
    /// Create a previously absent ref.
    Create,
    /// Change an existing ref, with ancestry already verified.
    Update {
        /// Whether the old object is an ancestor of the new object.
        fast_forward: bool,
    },
    /// Delete an existing ref.
    Delete,
}

/// One trusted changed-path record from a proposed receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathChange<'a> {
    /// A new path.
    Addition(&'a str),
    /// An existing path with changed content or mode.
    Modification(&'a str),
    /// A removed path.
    Deletion(&'a str),
    /// A rename or copy whose source and destination both require authority.
    Rename {
        /// Original repository-relative path.
        from: &'a str,
        /// New repository-relative path.
        to: &'a str,
    },
}

impl<'a> PathChange<'a> {
    const fn paths(self) -> [Option<&'a str>; 2] {
        match self {
            Self::Addition(path) | Self::Modification(path) | Self::Deletion(path) => {
                [Some(path), None]
            }
            Self::Rename { from, to } => [Some(from), Some(to)],
        }
    }
}

/// Trusted input for checking one atomic receive ref command.
#[derive(Debug, Clone, Copy)]
pub struct ReceiveUpdate<'a> {
    /// Fully qualified proposed ref.
    pub reference: &'a str,
    /// Creation, verified update, or deletion.
    pub transition: RefTransition,
    /// Complete path delta required by [`GitCapabilityScope::allows_receive`].
    pub changed_paths: &'a [PathChange<'a>],
}

/// SHA-256 digest of a normalized capability scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GitCapabilityHash([u8; 32]);

impl GitCapabilityHash {
    /// Restores a digest read from trusted immutable persistence.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for GitCapabilityHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Validation or canonicalization failure for a Git capability.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitCapabilityError {
    /// Repository ID was not canonical lowercase hyphenated UUID text.
    #[error("repository ID is not a canonical lowercase hyphenated UUID: {0:?}")]
    NonCanonicalRepositoryId(String),
    /// A ref glob was malformed or outside the strict grammar.
    #[error("invalid ref glob: {0}")]
    InvalidRefGlob(&'static str),
    /// A changed-path glob was malformed or outside the strict grammar.
    #[error("invalid changed-path glob: {0}")]
    InvalidChangedPathGlob(&'static str),
    /// A repository- or namespace-wide glob lacked explicit opt-in.
    #[error("broad glob requires explicit opt-in: {0:?}")]
    BroadGlobRequiresExplicitOptIn(String),
    /// A required collection was empty or exceeded its bound.
    #[error("invalid bounded collection {0}")]
    InvalidCollection(&'static str),
    /// Receive-only fields conflicted with the operation set.
    #[error("conflicting Git capability scope: {0}")]
    ConflictingScope(&'static str),
    /// An instance-owned Git authority attempted to exceed its release ceiling.
    #[error("Git capability binding broadens its release ceiling")]
    ScopeBroadening,
    /// Expiry was not a positive, whole Unix second.
    #[error("expiry must be a positive whole Unix second")]
    InvalidExpiry,
    /// A transfer limit was zero or exceeded a hard grammar ceiling.
    #[error("transfer limits must be non-zero and within hard ceilings")]
    InvalidTransferLimits,
    /// Canonical JSON serialization failed.
    #[error("canonical Git capability serialization failed: {0}")]
    CanonicalSerialization(serde_json::Error),
}

fn git_hash(bytes: &[u8]) -> GitCapabilityHash {
    let digest = Sha256::digest(bytes);
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&digest);
    GitCapabilityHash(hash)
}

const fn permission_is_attenuation(
    selected: RefMutationPermission,
    ceiling: RefMutationPermission,
) -> bool {
    matches!(selected, RefMutationPermission::Deny)
        || matches!(ceiling, RefMutationPermission::Allow)
}

const fn namespace_policy_is_attenuation(
    selected: RefNamespacePolicy,
    ceiling: RefNamespacePolicy,
) -> bool {
    permission_is_attenuation(selected.create, ceiling.create)
        && permission_is_attenuation(selected.update, ceiling.update)
        && permission_is_attenuation(selected.delete, ceiling.delete)
}

const fn update_policy_is_attenuation(selected: RefUpdatePolicy, ceiling: RefUpdatePolicy) -> bool {
    let branch_update_narrow =
        matches!(
            selected.branches.updates,
            BranchUpdatePolicy::FastForwardOnly
        ) || matches!(ceiling.branches.updates, BranchUpdatePolicy::AllowForce);
    branch_update_narrow
        && permission_is_attenuation(selected.branches.create, ceiling.branches.create)
        && permission_is_attenuation(selected.branches.delete, ceiling.branches.delete)
        && namespace_policy_is_attenuation(selected.tags, ceiling.tags)
        && namespace_policy_is_attenuation(selected.other, ceiling.other)
}

const fn limits_are_attenuation(selected: TransferLimits, ceiling: TransferLimits) -> bool {
    selected.request_bytes <= ceiling.request_bytes
        && selected.pack_bytes <= ceiling.pack_bytes
        && selected.object_count <= ceiling.object_count
        && selected.ref_updates <= ceiling.ref_updates
}

#[derive(Debug, Clone, Copy)]
enum RefKind {
    Branch,
    Tag,
    Other,
}

impl RefKind {
    fn of(reference: &str) -> Self {
        if reference.starts_with("refs/heads/") {
            Self::Branch
        } else if reference.starts_with("refs/tags/") {
            Self::Tag
        } else {
            Self::Other
        }
    }
}

fn normalize_bounded<T: Ord>(
    values: Vec<T>,
    name: &'static str,
) -> Result<Vec<T>, GitCapabilityError> {
    let normalized: Vec<_> = values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if normalized.is_empty() || normalized.len() > MAX_GLOBS {
        return Err(GitCapabilityError::InvalidCollection(name));
    }
    Ok(normalized)
}

fn normalize_optional_bounded<T: Ord>(values: Vec<T>) -> Result<Vec<T>, GitCapabilityError> {
    let normalized: Vec<_> = values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if normalized.len() > MAX_GLOBS {
        return Err(GitCapabilityError::InvalidCollection("changed_path_globs"));
    }
    Ok(normalized)
}

// Git reserves the lowercase `.lock` suffix specifically; this is not a host
// filesystem extension comparison.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn validate_ref_glob(value: &str) -> Result<(), GitCapabilityError> {
    validate_common(value, MAX_REF_GLOB_BYTES).map_err(GitCapabilityError::InvalidRefGlob)?;
    let segments: Vec<_> = value.split('/').collect();
    if segments.len() < 3 || segments[0] != "refs" || !is_literal_segment(segments[1]) {
        return Err(GitCapabilityError::InvalidRefGlob(
            "must be anchored below an explicit refs/<namespace>/ prefix",
        ));
    }
    if value.contains("@{")
        || segments.iter().any(|segment| {
            segment.ends_with('.')
                || segment.ends_with(".lock")
                || segment.starts_with('.')
                || segment
                    .chars()
                    .any(|character| matches!(character, ' ' | '~' | '^' | ':' | '?' | '['))
        })
    {
        return Err(GitCapabilityError::InvalidRefGlob(
            "contains a Git-ref-forbidden spelling",
        ));
    }
    Ok(())
}

fn validate_path_glob(value: &str) -> Result<(), GitCapabilityError> {
    validate_common(value, MAX_PATH_GLOB_BYTES).map_err(GitCapabilityError::InvalidChangedPathGlob)
}

fn validate_common(value: &str, max_bytes: usize) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > max_bytes {
        return Err("is empty or exceeds its byte bound");
    }
    if value.starts_with('/') || value.ends_with('/') || value.contains("//") {
        return Err("must be anchored and contain no empty segments");
    }
    if value
        .chars()
        .any(|character| character == '\\' || character.is_control())
    {
        return Err("contains a backslash or control character");
    }
    for segment in value.split('/') {
        if segment == "." || segment == ".." {
            return Err("contains a dot path segment");
        }
        if segment.contains("**") && segment != "**" {
            return Err("uses ** inside a segment");
        }
    }
    Ok(())
}

fn is_literal_segment(segment: &str) -> bool {
    !segment.contains('*')
}

fn is_broad_ref_glob(value: &str) -> bool {
    let mut segments = value.split('/');
    let _refs = segments.next();
    let _namespace = segments.next();
    matches!((segments.next(), segments.next()), (Some("*" | "**"), None))
}

fn validate_concrete_ref(value: &str) -> Result<(), GitCapabilityError> {
    validate_ref_glob(value)?;
    if value.contains('*') {
        return Err(GitCapabilityError::InvalidRefGlob(
            "a concrete ref cannot contain wildcards",
        ));
    }
    Ok(())
}

fn validate_concrete_path(value: &str) -> Result<(), GitCapabilityError> {
    validate_path_glob(value)?;
    if value.contains('*') {
        return Err(GitCapabilityError::InvalidChangedPathGlob(
            "a concrete path cannot contain wildcards",
        ));
    }
    Ok(())
}

fn glob_matches(pattern: &str, candidate: &str) -> bool {
    let pattern: Vec<_> = pattern.split('/').collect();
    let candidate: Vec<_> = candidate.split('/').collect();
    let mut reachable = vec![false; candidate.len() + 1];
    reachable[0] = true;
    for pattern_segment in pattern {
        if pattern_segment == "**" {
            for index in 1..=candidate.len() {
                reachable[index] = reachable[index] || reachable[index - 1];
            }
        } else {
            let mut next = vec![false; candidate.len() + 1];
            for index in 1..=candidate.len() {
                next[index] =
                    reachable[index - 1] && segment_matches(pattern_segment, candidate[index - 1]);
            }
            reachable = next;
        }
    }
    reachable[candidate.len()]
}

fn segment_matches(pattern: &str, candidate: &str) -> bool {
    let pattern: Vec<_> = pattern.chars().collect();
    let candidate: Vec<_> = candidate.chars().collect();
    let mut reachable = vec![false; candidate.len() + 1];
    reachable[0] = true;
    for character in pattern {
        if character == '*' {
            for index in 1..=candidate.len() {
                reachable[index] = reachable[index] || reachable[index - 1];
            }
        } else {
            for index in (1..=candidate.len()).rev() {
                reachable[index] = reachable[index - 1] && candidate[index - 1] == character;
            }
            reachable[0] = false;
        }
    }
    reachable[candidate.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn limits() -> TransferLimits {
        TransferLimits::new(1_024, 2_048, 100, 10).expect("valid limits")
    }

    fn receive_scope(
        refs: Vec<RefGlob>,
        paths: Vec<ChangedPathGlob>,
        policy: RefUpdatePolicy,
    ) -> GitCapabilityScope {
        GitCapabilityScope::new(GitCapabilityScopeInput {
            repository_id: RepositoryId::new(Uuid::from_u128(1)),
            operations: vec![GitOperation::Receive],
            ref_globs: refs,
            changed_path_globs: paths,
            update_policy: policy,
            expires_at_unix_seconds: 2_000_000_000,
            transfer_limits: limits(),
        })
        .expect("valid receive scope")
    }

    fn ceiling(
        refs: Vec<RefGlob>,
        paths: Vec<ChangedPathGlob>,
        update_policy: RefUpdatePolicy,
        transfer_limits: TransferLimits,
        exact_parent_required: bool,
    ) -> GitCapabilityCeiling {
        GitCapabilityCeiling::new(GitCapabilityCeilingInput {
            operations: vec![
                GitOperation::Discover,
                GitOperation::Fetch,
                GitOperation::Receive,
            ],
            ref_globs: refs,
            changed_path_globs: paths,
            update_policy,
            transfer_limits,
            exact_parent_required,
        })
        .expect("valid Git ceiling")
    }

    #[test]
    fn instance_git_authority_can_only_attenuate_release_ceiling() {
        let permissive = RefUpdatePolicy {
            branches: BranchRefPolicy {
                updates: BranchUpdatePolicy::AllowForce,
                create: RefMutationPermission::Allow,
                delete: RefMutationPermission::Allow,
            },
            ..RefUpdatePolicy::default()
        };
        let release = ceiling(
            vec![
                RefGlob::parse("refs/heads/content").expect("content ref"),
                RefGlob::parse("refs/heads/drafts").expect("draft ref"),
            ],
            vec![
                ChangedPathGlob::parse("content/**").expect("content path"),
                ChangedPathGlob::parse("drafts/**").expect("draft path"),
            ],
            permissive,
            TransferLimits::new(4_096, 8_192, 200, 20).expect("release limits"),
            false,
        );
        let selected = ceiling(
            vec![RefGlob::parse("refs/heads/content").expect("content ref")],
            vec![ChangedPathGlob::parse("content/**").expect("content path")],
            RefUpdatePolicy::default(),
            limits(),
            true,
        );
        assert!(selected.is_attenuation_of(&release));
        let bound = BoundGitCapability::new(
            RepositoryId::new(Uuid::from_u128(1)),
            selected.clone(),
            &release,
        )
        .expect("narrow binding");
        assert_eq!(bound.authority(), &selected);

        let broader_limits = ceiling(
            vec![RefGlob::parse("refs/heads/content").expect("content ref")],
            vec![ChangedPathGlob::parse("content/**").expect("content path")],
            RefUpdatePolicy::default(),
            TransferLimits::new(8_192, 8_192, 200, 20).expect("broader limits"),
            false,
        );
        assert!(!broader_limits.is_attenuation_of(&release));
        assert!(
            BoundGitCapability::new(
                RepositoryId::new(Uuid::from_u128(1)),
                broader_limits,
                &release,
            )
            .is_err()
        );
    }

    #[test]
    fn bound_hash_includes_exact_repository_identity() {
        let release = ceiling(
            vec![RefGlob::parse("refs/heads/content").expect("content ref")],
            vec![ChangedPathGlob::parse("content/**").expect("content path")],
            RefUpdatePolicy::default(),
            limits(),
            false,
        );
        let first = BoundGitCapability::new(
            RepositoryId::new(Uuid::from_u128(1)),
            release.clone(),
            &release,
        )
        .expect("first binding");
        let second = BoundGitCapability::new(
            RepositoryId::new(Uuid::from_u128(2)),
            release.clone(),
            &release,
        )
        .expect("second binding");
        assert_ne!(
            first.normalized_hash().expect("first hash"),
            second.normalized_hash().expect("second hash")
        );
    }

    #[test]
    fn repository_id_requires_canonical_text() {
        let canonical = "00000000-0000-0000-0000-000000000001";
        assert_eq!(
            canonical
                .parse::<RepositoryId>()
                .expect("canonical")
                .to_string(),
            canonical
        );
        assert!(
            "00000000000000000000000000000001"
                .parse::<RepositoryId>()
                .is_err()
        );
        assert!(
            "00000000-0000-0000-0000-00000000000A"
                .parse::<RepositoryId>()
                .is_err()
        );
    }

    #[test]
    fn strict_globs_reject_ambiguous_and_implicit_broad_forms() {
        for invalid in [
            "main",
            "refs/*/main",
            "refs/heads/a/**b",
            "refs/heads/a//b",
            "refs/heads/../main",
            "refs/heads/bad.lock",
            "refs/heads/a\\b",
        ] {
            assert!(RefGlob::parse(invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(RefGlob::parse("refs/heads/**").is_err());
        assert!(RefGlob::parse("refs/heads/*").is_err());
        assert!(RefGlob::parse_explicitly_broad("refs/heads/**").is_ok());
        assert!(ChangedPathGlob::parse("**").is_err());
        assert!(ChangedPathGlob::parse("*").is_err());
        assert!(ChangedPathGlob::parse_explicitly_broad("**").is_ok());
    }

    #[test]
    fn glob_matching_is_anchored_segmented_and_case_sensitive() {
        let refs = RefGlob::parse("refs/heads/release/*").expect("valid ref glob");
        assert!(refs.is_match("refs/heads/release/v1"));
        assert!(!refs.is_match("refs/heads/release/v1/patch"));
        assert!(!refs.is_match("refs/heads/Release/v1"));

        let paths =
            ChangedPathGlob::parse("sessions/**/message-*.json").expect("valid changed-path glob");
        assert!(paths.is_match("sessions/message-1.json"));
        assert!(paths.is_match("sessions/alice/2026/message-1.json"));
        assert!(!paths.is_match("archive/sessions/message-1.json"));
    }

    #[test]
    fn unicode_matching_is_exact_without_normalization_or_case_folding() {
        let glob = ChangedPathGlob::parse("résumés/café.md").expect("valid Unicode glob");
        assert!(glob.is_match("résumés/café.md"));
        assert!(!glob.is_match("résumés/cafe\u{301}.md"));
        assert!(!glob.is_match("RÉSUMÉS/café.md"));
    }

    #[test]
    fn normalization_sorts_deduplicates_and_hashes_stably() {
        let first = GitCapabilityScope::new(GitCapabilityScopeInput {
            repository_id: RepositoryId::new(Uuid::from_u128(1)),
            operations: vec![
                GitOperation::Fetch,
                GitOperation::Discover,
                GitOperation::Fetch,
            ],
            ref_globs: vec![
                RefGlob::parse("refs/tags/v*").expect("tag glob"),
                RefGlob::parse("refs/heads/main").expect("branch glob"),
                RefGlob::parse("refs/tags/v*").expect("tag glob"),
            ],
            changed_path_globs: Vec::new(),
            update_policy: RefUpdatePolicy::default(),
            expires_at_unix_seconds: 2_000_000_000,
            transfer_limits: limits(),
        })
        .expect("valid scope");
        let second = GitCapabilityScope::new(GitCapabilityScopeInput {
            repository_id: RepositoryId::new(Uuid::from_u128(1)),
            operations: vec![GitOperation::Discover, GitOperation::Fetch],
            ref_globs: vec![
                RefGlob::parse("refs/heads/main").expect("branch glob"),
                RefGlob::parse("refs/tags/v*").expect("tag glob"),
            ],
            changed_path_globs: Vec::new(),
            update_policy: RefUpdatePolicy::default(),
            expires_at_unix_seconds: 2_000_000_000,
            transfer_limits: limits(),
        })
        .expect("valid scope");

        assert_eq!(first, second);
        assert_eq!(
            first.normalized_hash().expect("hash"),
            second.normalized_hash().expect("hash")
        );
        assert_eq!(
            first.canonical_json().expect("JSON"),
            second.canonical_json().expect("JSON")
        );
    }

    #[test]
    fn receive_scope_requires_paths_and_rejects_read_only_update_policy() {
        let common = |operations, paths, policy| {
            GitCapabilityScope::new(GitCapabilityScopeInput {
                repository_id: RepositoryId::new(Uuid::from_u128(1)),
                operations,
                ref_globs: vec![RefGlob::parse("refs/heads/main").expect("ref glob")],
                changed_path_globs: paths,
                update_policy: policy,
                expires_at_unix_seconds: 2_000_000_000,
                transfer_limits: limits(),
            })
        };
        assert!(
            common(
                vec![GitOperation::Receive],
                Vec::new(),
                RefUpdatePolicy::default()
            )
            .is_err()
        );
        let non_default = RefUpdatePolicy {
            branches: BranchRefPolicy {
                create: RefMutationPermission::Allow,
                ..BranchRefPolicy::default()
            },
            ..RefUpdatePolicy::default()
        };
        assert!(common(vec![GitOperation::Fetch], Vec::new(), non_default).is_err());
    }

    #[test]
    fn receive_checks_transitions_and_every_rename_endpoint() {
        let policy = RefUpdatePolicy {
            branches: BranchRefPolicy {
                create: RefMutationPermission::Allow,
                ..BranchRefPolicy::default()
            },
            ..RefUpdatePolicy::default()
        };
        let scope = receive_scope(
            vec![RefGlob::parse("refs/heads/session-*").expect("ref glob")],
            vec![ChangedPathGlob::parse("sessions/**").expect("path glob")],
            policy,
        );
        let allowed = [PathChange::Addition("sessions/a/1.json")];
        assert!(scope.allows_receive(&ReceiveUpdate {
            reference: "refs/heads/session-a",
            transition: RefTransition::Create,
            changed_paths: &allowed,
        }));

        let escaped_rename = [PathChange::Rename {
            from: "sessions/a/1.json",
            to: "private/1.json",
        }];
        assert!(!scope.allows_receive(&ReceiveUpdate {
            reference: "refs/heads/session-a",
            transition: RefTransition::Update { fast_forward: true },
            changed_paths: &escaped_rename,
        }));
        assert!(!scope.allows_receive(&ReceiveUpdate {
            reference: "refs/heads/session-a",
            transition: RefTransition::Update {
                fast_forward: false,
            },
            changed_paths: &allowed,
        }));
    }

    #[test]
    fn tag_and_deletion_rules_are_independent() {
        let policy = RefUpdatePolicy {
            tags: RefNamespacePolicy {
                create: RefMutationPermission::Allow,
                ..RefNamespacePolicy::default()
            },
            ..RefUpdatePolicy::default()
        };
        let scope = receive_scope(
            vec![RefGlob::parse("refs/tags/release-*").expect("ref glob")],
            vec![ChangedPathGlob::parse("release/**").expect("path glob")],
            policy,
        );
        let changes = [PathChange::Addition("release/manifest.json")];
        assert!(scope.allows_receive(&ReceiveUpdate {
            reference: "refs/tags/release-1",
            transition: RefTransition::Create,
            changed_paths: &changes,
        }));
        assert!(!scope.allows_receive(&ReceiveUpdate {
            reference: "refs/tags/release-1",
            transition: RefTransition::Delete,
            changed_paths: &changes,
        }));
    }

    #[test]
    fn expiry_is_exclusive_and_transfer_limits_are_bounded() {
        assert!(TransferLimits::new(0, 1, 1, 1).is_err());
        assert!(TransferLimits::new(1, MAX_PACK_BYTES + 1, 1, 1).is_err());
        let scope = GitCapabilityScope::new(GitCapabilityScopeInput {
            repository_id: RepositoryId::new(Uuid::from_u128(1)),
            operations: vec![GitOperation::Fetch],
            ref_globs: vec![RefGlob::parse("refs/heads/main").expect("ref glob")],
            changed_path_globs: Vec::new(),
            update_policy: RefUpdatePolicy::default(),
            expires_at_unix_seconds: 100,
            transfer_limits: limits(),
        })
        .expect("scope");
        assert!(scope.is_active_at(99));
        assert!(!scope.is_active_at(100));
    }

    proptest! {
        #[test]
        fn normalization_is_permutation_and_duplicate_invariant(
            operation_indexes in prop::collection::vec(0_u8..2, 1..40),
            ref_indexes in prop::collection::vec(0_u8..4, 1..40),
        ) {
            let operation_for = |index| match index {
                0 => GitOperation::Discover,
                _ => GitOperation::Fetch,
            };
            let ref_for = |index| match index {
                0 => RefGlob::parse("refs/heads/main").expect("glob"),
                1 => RefGlob::parse("refs/heads/release-*").expect("glob"),
                2 => RefGlob::parse("refs/tags/v*").expect("glob"),
                _ => RefGlob::parse("refs/notes/build-*").expect("glob"),
            };
            let build = |operations: Vec<_>, refs: Vec<_>| {
                GitCapabilityScope::new(GitCapabilityScopeInput {
                    repository_id: RepositoryId::new(Uuid::from_u128(42)),
                    operations,
                    ref_globs: refs,
                    changed_path_globs: Vec::new(),
                    update_policy: RefUpdatePolicy::default(),
                    expires_at_unix_seconds: 2_000_000_000,
                    transfer_limits: limits(),
                }).expect("scope")
            };
            let mut operations: Vec<_> = operation_indexes.into_iter().map(operation_for).collect();
            let mut refs: Vec<_> = ref_indexes.into_iter().map(ref_for).collect();
            let first = build(operations.clone(), refs.clone());
            operations.reverse();
            refs.reverse();
            operations.extend(operations.clone());
            refs.extend(refs.clone());
            let second = build(operations, refs);
            prop_assert_eq!(first.normalized_hash().expect("hash"), second.normalized_hash().expect("hash"));
        }

        #[test]
        fn literal_path_globs_match_only_identical_unicode_scalar_text(
            segments in prop::collection::vec("[a-zA-Z0-9é]{1,12}", 1..6),
        ) {
            let path = segments.join("/");
            let glob = ChangedPathGlob::parse(path.clone()).expect("literal glob");
            let prefixed = format!("prefix/{path}");
            prop_assert!(glob.is_match(&path));
            prop_assert!(!glob.is_match(&prefixed));
        }

        #[test]
        fn star_never_crosses_a_path_segment(
            left in "[a-z]{1,12}",
            right in "[a-z]{1,12}",
        ) {
            let glob = ChangedPathGlob::parse("root/*").expect("glob");
            let one_segment = format!("root/{left}");
            let two_segments = format!("root/{left}/{right}");
            prop_assert!(glob.is_match(&one_segment));
            prop_assert!(!glob.is_match(&two_segments));
        }
    }
}

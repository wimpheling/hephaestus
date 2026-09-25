use serde::Serialize;

use crate::{
    ChangedPathGlob, GRAMMAR_VERSION, GitCapabilityError, GitCapabilityHash, GitOperation,
    PathChange, ReceiveUpdate, RefGlob, RefUpdatePolicy, RepositoryId, TransferLimits,
    errors::{RefKind, normalize_bounded, normalize_optional_bounded},
    git_hash::git_hash,
    policies::policy_permits,
    transitions::paths,
};

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
            && policy_permits(
                self.update_policy,
                RefKind::of(update.reference),
                update.transition,
            )
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
        paths(*change).into_iter().flatten().all(|path| {
            self.changed_path_globs
                .iter()
                .any(|glob| glob.is_match(path))
        })
    }
}

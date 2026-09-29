use serde::Serialize;

use crate::{
    ChangedPathGlob, GRAMMAR_VERSION, GitCapabilityError, GitCapabilityHash, GitOperation, RefGlob,
    RefUpdatePolicy, RepositoryId, TransferLimits,
    errors::{
        limits_are_attenuation, normalize_bounded, normalize_optional_bounded,
        update_policy_is_attenuation,
    },
    git_hash::git_hash,
};

/// Unnormalized release- or instance-owned Git authority rules.
///
/// Unlike [`crate::GitCapabilityScopeInput`], these rules do not contain an exact
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

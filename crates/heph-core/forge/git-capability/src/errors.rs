use std::collections::BTreeSet;

use crate::{
    BranchUpdatePolicy, MAX_GLOBS, RefMutationPermission, RefNamespacePolicy, RefUpdatePolicy,
    TransferLimits,
};

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
pub const fn permission_is_attenuation(
    selected: RefMutationPermission,
    ceiling: RefMutationPermission,
) -> bool {
    matches!(selected, RefMutationPermission::Deny)
        || matches!(ceiling, RefMutationPermission::Allow)
}

pub const fn namespace_policy_is_attenuation(
    selected: RefNamespacePolicy,
    ceiling: RefNamespacePolicy,
) -> bool {
    permission_is_attenuation(selected.create, ceiling.create)
        && permission_is_attenuation(selected.update, ceiling.update)
        && permission_is_attenuation(selected.delete, ceiling.delete)
}

pub const fn update_policy_is_attenuation(
    selected: RefUpdatePolicy,
    ceiling: RefUpdatePolicy,
) -> bool {
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

pub const fn limits_are_attenuation(selected: TransferLimits, ceiling: TransferLimits) -> bool {
    selected.request_bytes() <= ceiling.request_bytes()
        && selected.pack_bytes() <= ceiling.pack_bytes()
        && selected.object_count() <= ceiling.object_count()
        && selected.ref_updates() <= ceiling.ref_updates()
}

#[derive(Debug, Clone, Copy)]
pub enum RefKind {
    Branch,
    Tag,
    Other,
}

impl RefKind {
    pub fn of(reference: &str) -> Self {
        if reference.starts_with("refs/heads/") {
            Self::Branch
        } else if reference.starts_with("refs/tags/") {
            Self::Tag
        } else {
            Self::Other
        }
    }
}

pub fn normalize_bounded<T: Ord>(
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

pub fn normalize_optional_bounded<T: Ord>(values: Vec<T>) -> Result<Vec<T>, GitCapabilityError> {
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

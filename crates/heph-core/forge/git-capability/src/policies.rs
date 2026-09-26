use serde::{Deserialize, Serialize};

use crate::{RefTransition, errors::RefKind};

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

pub fn policy_permits(policy: RefUpdatePolicy, kind: RefKind, transition: RefTransition) -> bool {
    policy.permits(kind, transition)
}

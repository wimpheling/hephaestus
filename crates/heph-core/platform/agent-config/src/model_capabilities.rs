//! Capability and typed Git authority declarations.
use capability_domain::{
    CapabilityError, CapabilityOperation, CapabilityRequirement, CapabilityRequirementId,
    CapabilityResourceKind, CapabilitySlotKey,
};
use git_capability_domain::{
    BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitCapabilityError, GitOperation, RefGlob, RefMutationPermission,
    RefNamespacePolicy, RefUpdatePolicy, TransferLimits,
};
use serde::{Deserialize, Serialize};

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

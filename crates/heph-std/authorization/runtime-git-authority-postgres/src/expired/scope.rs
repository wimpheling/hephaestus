//! Pure immutable scope decoding; never a live authentication timestamp override.

use crate::{parse_operations, permission, persisted_hash, storage};
use git_capability_domain::{
    BoundGitCapability, BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, RefGlob, RefNamespacePolicy, RefUpdatePolicy, RepositoryId,
    TransferLimits,
};
use runtime_authority::ExpiredRuntimeHandoff;
use runtime_git_authority::RuntimeGitAuthorityError;
use uuid::Uuid;

// These are independent persisted permission bits, not adapter lifecycle states.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(super) struct Shape {
    pub(super) grammar_version: i16,
    pub(super) git_operations: Vec<String>,
    pub(super) ref_globs: Vec<String>,
    pub(super) changed_path_globs: Vec<String>,
    pub(super) branch_update_policy: String,
    pub(super) branch_create: bool,
    pub(super) branch_delete: bool,
    pub(super) tag_create: bool,
    pub(super) tag_update: bool,
    pub(super) tag_delete: bool,
    pub(super) other_create: bool,
    pub(super) other_update: bool,
    pub(super) other_delete: bool,
    pub(super) request_bytes: i64,
    pub(super) pack_bytes: i64,
    pub(super) object_count: i32,
    pub(super) ref_updates: i32,
    pub(super) exact_parent_required: bool,
    pub(super) normalized_hash: Vec<u8>,
}

#[derive(sqlx::FromRow)]
pub(super) struct StoredSnapshot {
    pub(super) snapshot_id: Uuid,
    pub(super) instance_revision_id: Uuid,
    pub(super) binding_id: Uuid,
    pub(super) repository_id: Uuid,
    pub(super) expected_parent: Option<String>,
    #[sqlx(flatten)]
    pub(super) shape: Shape,
}
#[derive(sqlx::FromRow)]
pub(super) struct StoredBinding {
    pub(super) binding_id: Uuid,
    pub(super) instance_revision_id: Uuid,
    pub(super) requirement_id: Uuid,
    #[sqlx(flatten)]
    pub(super) shape: Shape,
}
#[derive(sqlx::FromRow)]
pub(super) struct ReleaseRequirement {
    pub(super) requirement_id: Uuid,
    pub(super) release_agent_id: Uuid,
    pub(super) requirement_hash: Vec<u8>,
    pub(super) declared_operations: Vec<String>,
    pub(super) resource_kind: String,
    #[sqlx(flatten)]
    pub(super) shape: Shape,
}

pub(super) fn qualify(
    snapshot: &StoredSnapshot,
    binding: &StoredBinding,
    release: &ReleaseRequirement,
    original: &ExpiredRuntimeHandoff,
) -> Result<(), RuntimeGitAuthorityError> {
    let generic = original
        .snapshot()
        .bindings()
        .find(|item| item.id().as_uuid() == snapshot.binding_id)
        .ok_or(RuntimeGitAuthorityError::IdentityMismatch)?;
    if snapshot.snapshot_id != original.snapshot().id().as_uuid()
        || snapshot.instance_revision_id != original.pins().revision()
        || binding.binding_id != snapshot.binding_id
        || binding.instance_revision_id != snapshot.instance_revision_id
        || generic.resource().kind.as_str() != "repository"
        || generic.resource().id != snapshot.repository_id
        || snapshot.repository_id.is_nil()
        || generic.requirement_id().as_uuid() != binding.requirement_id
        || release.requirement_id != binding.requirement_id
        || release.release_agent_id != original.pins().release_agent()
        || release.resource_kind != "repository"
        || release.requirement_hash.as_slice() != generic.requirement_hash().as_bytes()
        || snapshot.shape != binding.shape
        || snapshot.shape.exact_parent_required != snapshot.expected_parent.is_some()
    {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    }
    release
        .shape
        .generic_permissions(&release.declared_operations)?;
    binding.shape.generic_permissions(
        &generic
            .granted_operations()
            .map(|op| op.as_str().to_owned())
            .collect::<Vec<_>>(),
    )?;
    let ceiling = release.shape.decode()?;
    if ceiling.normalized_hash().map_err(storage)?
        != persisted_hash(&release.shape.normalized_hash)?
    {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    }
    let authority = binding.shape.decode()?;
    let bound = BoundGitCapability::new(
        RepositoryId::new(snapshot.repository_id),
        authority,
        &ceiling,
    )
    .map_err(storage)?;
    if bound.normalized_hash().map_err(storage)? != persisted_hash(&binding.shape.normalized_hash)?
    {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    }
    Ok(())
}

impl Shape {
    pub(super) fn decode(&self) -> Result<GitCapabilityCeiling, RuntimeGitAuthorityError> {
        if self.grammar_version
            != i16::try_from(git_capability_domain::GRAMMAR_VERSION).map_err(storage)?
        {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
        let ceiling = GitCapabilityCeiling::new(GitCapabilityCeilingInput {
            operations: parse_operations(&self.git_operations)?,
            ref_globs: self
                .ref_globs
                .iter()
                .cloned()
                .map(RefGlob::parse_explicitly_broad)
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?,
            changed_path_globs: self
                .changed_path_globs
                .iter()
                .cloned()
                .map(ChangedPathGlob::parse_explicitly_broad)
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?,
            update_policy: self.policy()?,
            transfer_limits: TransferLimits::new(
                u64::try_from(self.request_bytes).map_err(storage)?,
                u64::try_from(self.pack_bytes).map_err(storage)?,
                u32::try_from(self.object_count).map_err(storage)?,
                u16::try_from(self.ref_updates).map_err(storage)?,
            )
            .map_err(storage)?,
            exact_parent_required: self.exact_parent_required,
        })
        .map_err(storage)?;
        // Do not silently sort away duplicate or permuted raw persisted arrays.
        if !self.git_operations.iter().map(String::as_str).eq(ceiling
            .operations()
            .iter()
            .copied()
            .map(crate::operation_name))
            || !self
                .ref_globs
                .iter()
                .map(String::as_str)
                .eq(ceiling.ref_globs().iter().map(RefGlob::as_str))
            || !self
                .changed_path_globs
                .iter()
                .map(String::as_str)
                .eq(ceiling
                    .changed_path_globs()
                    .iter()
                    .map(ChangedPathGlob::as_str))
        {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
        Ok(ceiling)
    }

    pub(super) fn generic_permissions(
        &self,
        operations: &[String],
    ) -> Result<(), RuntimeGitAuthorityError> {
        let has = |operation: &str| operations.iter().any(|value| value == operation);
        if ((self
            .git_operations
            .iter()
            .any(|op| op == "discover" || op == "fetch"))
            && !has("git_read"))
            || (self.git_operations.iter().any(|op| op == "receive") && !has("update_ref"))
            || (self.branch_update_policy == "allow_force" && !has("force_update_ref"))
            || (self.branch_create && !has("create_ref"))
            || (self.branch_delete && !has("delete_ref"))
            || (self.tag_create && !has("create_tag"))
            || (self.tag_update && !has("force_update_ref"))
            || (self.tag_delete && !has("delete_tag"))
            || (self.other_create && !has("create_ref"))
            || (self.other_update && !has("force_update_ref"))
            || (self.other_delete && !has("delete_ref"))
        {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
        Ok(())
    }

    fn policy(&self) -> Result<RefUpdatePolicy, RuntimeGitAuthorityError> {
        Ok(RefUpdatePolicy {
            branches: BranchRefPolicy {
                updates: match self.branch_update_policy.as_str() {
                    "fast_forward_only" => BranchUpdatePolicy::FastForwardOnly,
                    "allow_force" => BranchUpdatePolicy::AllowForce,
                    _ => return Err(RuntimeGitAuthorityError::IdentityMismatch),
                },
                create: permission(self.branch_create),
                delete: permission(self.branch_delete),
            },
            tags: RefNamespacePolicy {
                create: permission(self.tag_create),
                update: permission(self.tag_update),
                delete: permission(self.tag_delete),
            },
            other: RefNamespacePolicy {
                create: permission(self.other_create),
                update: permission(self.other_update),
                delete: permission(self.other_delete),
            },
        })
    }
}

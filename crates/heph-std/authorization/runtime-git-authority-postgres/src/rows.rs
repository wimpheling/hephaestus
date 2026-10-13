//! Existing stored/live row decoding and canonical scope conversion.
use super::storage;
use capability_domain::{AuthorizationSnapshotId, RuntimeCredentialGeneration, RuntimeSessionId};
use git_capability_domain::{
    BoundGitCapability, BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitCapabilityHash, GitCapabilityScope, GitCapabilityScopeInput,
    GitOperation, RefGlob, RefMutationPermission, RefNamespacePolicy, RefUpdatePolicy,
    RepositoryId, TransferLimits,
};
use runtime_git_authority::{
    AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError, StoredRuntimeGitCredential,
};
use sqlx::FromRow;
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(FromRow)]
pub struct StoredRow {
    pub(super) runtime_session_id: Uuid,
    pub(super) authorization_snapshot_id: Uuid,
    pub(super) binding_id: Uuid,
    pub(super) repository_id: Uuid,
    pub(super) scope_hash: Vec<u8>,
    pub(super) issuance_generation: i64,
    pub(super) expires_at: OffsetDateTime,
}

impl TryFrom<StoredRow> for StoredRuntimeGitCredential {
    type Error = RuntimeGitAuthorityError;

    fn try_from(row: StoredRow) -> Result<Self, Self::Error> {
        Ok(Self {
            runtime_session_id: RuntimeSessionId::from_uuid(row.runtime_session_id),
            authorization_snapshot_id: AuthorizationSnapshotId::from_uuid(
                row.authorization_snapshot_id,
            ),
            binding_id: row.binding_id,
            repository_id: RepositoryId::new(row.repository_id),
            scope_hash: persisted_hash(&row.scope_hash)?,
            generation: RuntimeCredentialGeneration::new(
                u64::try_from(row.issuance_generation).map_err(storage)?,
            )
            .map_err(storage)?,
            expires_at: row.expires_at,
        })
    }
}

// This private row mirrors normalized SQL transition columns; conversion into
// domain enums happens immediately at the adapter boundary.
#[allow(clippy::struct_excessive_bools)]
#[derive(FromRow)]
pub struct AuthenticatedRow {
    pub(super) runtime_session_id: Uuid,
    pub(super) authorization_snapshot_id: Uuid,
    pub(super) binding_id: Uuid,
    pub(super) repository_id: Uuid,
    pub(super) scope_hash: Vec<u8>,
    pub(super) issuance_generation: i64,
    pub(super) expires_at: OffsetDateTime,
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
    pub(super) expected_parent: Option<String>,
    pub(super) evaluated_at: OffsetDateTime,
}

impl TryFrom<AuthenticatedRow> for AuthenticatedRuntimeGitAuthority {
    type Error = RuntimeGitAuthorityError;

    fn try_from(row: AuthenticatedRow) -> Result<Self, Self::Error> {
        if row.grammar_version
            != i16::try_from(git_capability_domain::GRAMMAR_VERSION).unwrap_or(-1)
            || row.issuance_generation <= 0
            || row.evaluated_at >= row.expires_at
        {
            return Err(RuntimeGitAuthorityError::Persistence);
        }
        let repository_id = RepositoryId::new(row.repository_id);
        let operations = parse_operations(&row.git_operations)?;
        let update_policy = update_policy(&row)?;
        let ref_globs = row
            .ref_globs
            .into_iter()
            .map(RefGlob::parse_explicitly_broad)
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        let changed_path_globs = row
            .changed_path_globs
            .into_iter()
            .map(ChangedPathGlob::parse_explicitly_broad)
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        let transfer_limits = TransferLimits::new(
            u64::try_from(row.request_bytes).map_err(storage)?,
            u64::try_from(row.pack_bytes).map_err(storage)?,
            u32::try_from(row.object_count).map_err(storage)?,
            u16::try_from(row.ref_updates).map_err(storage)?,
        )
        .map_err(storage)?;
        let ceiling = GitCapabilityCeiling::new(GitCapabilityCeilingInput {
            operations: operations.clone(),
            ref_globs: ref_globs.clone(),
            changed_path_globs: changed_path_globs.clone(),
            update_policy,
            transfer_limits,
            exact_parent_required: row.expected_parent.is_some(),
        })
        .map_err(storage)?;
        let binding =
            BoundGitCapability::new(repository_id, ceiling.clone(), &ceiling).map_err(storage)?;
        let scope_hash = persisted_hash(&row.scope_hash)?;
        if binding.normalized_hash().map_err(storage)? != scope_hash {
            return Err(RuntimeGitAuthorityError::Persistence);
        }
        let scope = GitCapabilityScope::new(GitCapabilityScopeInput {
            repository_id,
            operations,
            ref_globs,
            changed_path_globs,
            update_policy,
            expires_at_unix_seconds: row.expires_at.unix_timestamp(),
            transfer_limits,
        })
        .map_err(storage)?;
        Ok(Self {
            runtime_session_id: RuntimeSessionId::from_uuid(row.runtime_session_id),
            authorization_snapshot_id: AuthorizationSnapshotId::from_uuid(
                row.authorization_snapshot_id,
            ),
            binding_id: row.binding_id,
            scope_hash,
            expected_parent: row.expected_parent,
            scope: Arc::new(scope),
            evaluated_at: row.evaluated_at,
        })
    }
}

pub fn parse_operations(values: &[String]) -> Result<Vec<GitOperation>, RuntimeGitAuthorityError> {
    values
        .iter()
        .map(|value| match value.as_str() {
            "discover" => Ok(GitOperation::Discover),
            "fetch" => Ok(GitOperation::Fetch),
            "receive" => Ok(GitOperation::Receive),
            _ => Err(RuntimeGitAuthorityError::Persistence),
        })
        .collect()
}

fn update_policy(row: &AuthenticatedRow) -> Result<RefUpdatePolicy, RuntimeGitAuthorityError> {
    Ok(RefUpdatePolicy {
        branches: BranchRefPolicy {
            updates: match row.branch_update_policy.as_str() {
                "fast_forward_only" => BranchUpdatePolicy::FastForwardOnly,
                "allow_force" => BranchUpdatePolicy::AllowForce,
                _ => return Err(RuntimeGitAuthorityError::Persistence),
            },
            create: permission(row.branch_create),
            delete: permission(row.branch_delete),
        },
        tags: RefNamespacePolicy {
            create: permission(row.tag_create),
            update: permission(row.tag_update),
            delete: permission(row.tag_delete),
        },
        other: RefNamespacePolicy {
            create: permission(row.other_create),
            update: permission(row.other_update),
            delete: permission(row.other_delete),
        },
    })
}

pub const fn permission(value: bool) -> RefMutationPermission {
    if value {
        RefMutationPermission::Allow
    } else {
        RefMutationPermission::Deny
    }
}

pub fn persisted_hash(bytes: &[u8]) -> Result<GitCapabilityHash, RuntimeGitAuthorityError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| RuntimeGitAuthorityError::Persistence)?;
    Ok(GitCapabilityHash::from_bytes(bytes))
}

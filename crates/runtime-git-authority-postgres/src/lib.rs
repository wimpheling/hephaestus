//! `PostgreSQL` persistence and live authentication for runtime Git bearers.

use async_trait::async_trait;
use capability_domain::{AuthorizationSnapshotId, RuntimeCredentialGeneration, RuntimeSessionId};
use git_capability_domain::{
    BoundGitCapability, BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitCapabilityHash, GitCapabilityScope, GitCapabilityScopeInput,
    GitOperation, RefGlob, RefMutationPermission, RefNamespacePolicy, RefUpdatePolicy,
    RepositoryId, TransferLimits,
};
use runtime_git_authority::{
    AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError, RuntimeGitCredentialHash,
    RuntimeGitCredentialRepository, StoredRuntimeGitCredential,
};
use sqlx::{FromRow, PgPool};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

/// `PostgreSQL` runtime Git credential adapter.
#[derive(Clone)]
pub struct PgRuntimeGitCredentialRepository {
    pool: PgPool,
}

impl PgRuntimeGitCredentialRepository {
    /// Creates an adapter using a control-plane connection pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RuntimeGitCredentialRepository for PgRuntimeGitCredentialRepository {
    async fn find(
        &self,
        session_id: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeGitCredential>, RuntimeGitAuthorityError> {
        sqlx::query_as::<_, StoredRow>(
            "SELECT runtime_session_id, authorization_snapshot_id, binding_id,
                    repository_id, scope_hash, issuance_generation, expires_at
             FROM runtime_git_credentials WHERE runtime_session_id = $1",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn create(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        credential_hash: RuntimeGitCredentialHash,
    ) -> Result<StoredRuntimeGitCredential, RuntimeGitAuthorityError> {
        let generation = i64::try_from(generation.get()).map_err(storage)?;
        sqlx::query_as::<_, StoredRow>(
            "INSERT INTO runtime_git_credentials
                (runtime_session_id, authorization_snapshot_id, binding_id,
                 repository_id, scope_hash, issuance_generation,
                 credential_hash, expires_at)
             SELECT session.id, session.snapshot_id, git.binding_id,
                    git.repository_id, git.normalized_hash,
                    session.issuance_generation, $3, session.expires_at
             FROM runtime_authority_sessions AS session
             JOIN run_git_authority_snapshots AS git
               ON git.snapshot_id = session.snapshot_id
             WHERE session.id = $1
               AND session.issuance_generation = $2
               AND session.status = 'pending_handoff'
               AND session.expires_at > now()
             RETURNING runtime_session_id, authorization_snapshot_id,
                       binding_id, repository_id, scope_hash,
                       issuance_generation, expires_at",
        )
        .bind(session_id.as_uuid())
        .bind(generation)
        .bind(credential_hash.as_bytes().as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(RuntimeGitAuthorityError::NotFound)?
        .try_into()
    }

    async fn authenticate(
        &self,
        credential_hash: RuntimeGitCredentialHash,
        repository_id: RepositoryId,
        operation: GitOperation,
        _evaluated_at: OffsetDateTime,
    ) -> Result<AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError> {
        let row = sqlx::query_as::<_, AuthenticatedRow>(
            "SELECT * FROM authenticate_runtime_git_credential($1, $2, $3)",
        )
        .bind(credential_hash.as_bytes().as_slice())
        .bind(repository_id.as_uuid())
        .bind(operation_name(operation))
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(RuntimeGitAuthorityError::NotFound)?;
        row.try_into()
    }
}

#[derive(FromRow)]
struct StoredRow {
    runtime_session_id: Uuid,
    authorization_snapshot_id: Uuid,
    binding_id: Uuid,
    repository_id: Uuid,
    scope_hash: Vec<u8>,
    issuance_generation: i64,
    expires_at: OffsetDateTime,
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
struct AuthenticatedRow {
    runtime_session_id: Uuid,
    authorization_snapshot_id: Uuid,
    binding_id: Uuid,
    repository_id: Uuid,
    scope_hash: Vec<u8>,
    issuance_generation: i64,
    expires_at: OffsetDateTime,
    grammar_version: i16,
    git_operations: Vec<String>,
    ref_globs: Vec<String>,
    changed_path_globs: Vec<String>,
    branch_update_policy: String,
    branch_create: bool,
    branch_delete: bool,
    tag_create: bool,
    tag_update: bool,
    tag_delete: bool,
    other_create: bool,
    other_update: bool,
    other_delete: bool,
    request_bytes: i64,
    pack_bytes: i64,
    object_count: i32,
    ref_updates: i32,
    expected_parent: Option<String>,
    evaluated_at: OffsetDateTime,
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

fn parse_operations(values: &[String]) -> Result<Vec<GitOperation>, RuntimeGitAuthorityError> {
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

const fn permission(value: bool) -> RefMutationPermission {
    if value {
        RefMutationPermission::Allow
    } else {
        RefMutationPermission::Deny
    }
}

const fn operation_name(operation: GitOperation) -> &'static str {
    match operation {
        GitOperation::Discover => "discover",
        GitOperation::Fetch => "fetch",
        GitOperation::Receive => "receive",
    }
}

fn persisted_hash(bytes: &[u8]) -> Result<GitCapabilityHash, RuntimeGitAuthorityError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| RuntimeGitAuthorityError::Persistence)?;
    Ok(GitCapabilityHash::from_bytes(bytes))
}

fn storage(_error: impl std::fmt::Display) -> RuntimeGitAuthorityError {
    RuntimeGitAuthorityError::Persistence
}

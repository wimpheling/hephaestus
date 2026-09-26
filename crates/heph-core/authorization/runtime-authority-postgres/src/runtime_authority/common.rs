use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId, RuntimeSessionStatus};
use runtime_authority::{RuntimeAuthorityError, StoredRuntimeSession};
use sqlx::{FromRow, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

// Runtime Git authority is copied only from the release-owned publication
// binding. The database trigger verifies the complete copy and trigger parent.
pub(super) async fn insert_runtime_git_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    revision_id: Uuid,
    run_id: Uuid,
) -> Result<(), RuntimeAuthorityError> {
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
            (snapshot_id, instance_revision_id, binding_id, repository_id,
             grammar_version, git_operations, ref_globs, changed_path_globs,
             branch_update_policy, branch_create, branch_delete, tag_create,
             tag_update, tag_delete, other_create, other_update, other_delete,
             request_bytes, pack_bytes, object_count, ref_updates,
             exact_parent_required, expected_parent, normalized_hash)
         SELECT $1, $2, binding.binding_id, generic.resource_id,
                binding.grammar_version, binding.git_operations,
                binding.ref_globs, binding.changed_path_globs,
                binding.branch_update_policy, binding.branch_create,
                binding.branch_delete, binding.tag_create, binding.tag_update,
                binding.tag_delete, binding.other_create, binding.other_update,
                binding.other_delete, binding.request_bytes, binding.pack_bytes,
                binding.object_count, binding.ref_updates,
                binding.exact_parent_required,
                CASE WHEN binding.exact_parent_required
                     THEN provenance.target_commit ELSE NULL END,
                binding.normalized_hash
         FROM agent_instance_revisions AS revision
         JOIN agent_git_capability_bindings AS binding
           ON binding.binding_id = revision.publication_repository_binding_id
          AND binding.instance_revision_id = revision.id
         JOIN agent_capability_bindings AS generic
           ON generic.id = binding.binding_id
          AND generic.instance_revision_id = binding.instance_revision_id
         LEFT JOIN run_instance_provenance AS provenance ON provenance.run_id = $3
         WHERE revision.id = $2",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(run_id)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}

#[derive(FromRow)]
pub(super) struct SessionRow {
    pub(super) id: Uuid,
    pub(super) snapshot_id: Uuid,
    pub(super) identity_hash: Vec<u8>,
    pub(super) issuance_generation: i64,
    pub(super) status: String,
    pub(super) issued_at: OffsetDateTime,
    pub(super) expires_at: OffsetDateTime,
    pub(super) acknowledged_at: Option<OffsetDateTime>,
    pub(super) revoked_at: Option<OffsetDateTime>,
}

impl TryFrom<SessionRow> for StoredRuntimeSession {
    type Error = RuntimeAuthorityError;

    fn try_from(row: SessionRow) -> Result<Self, Self::Error> {
        let identity_hash: [u8; 32] = row
            .identity_hash
            .try_into()
            .map_err(|_| RuntimeAuthorityError::Persistence)?;
        Ok(Self {
            id: RuntimeSessionId::from_uuid(row.id),
            snapshot_id: capability_domain::AuthorizationSnapshotId::from_uuid(row.snapshot_id),
            identity_hash: capability_domain::AuthorityHash::from_bytes(identity_hash),
            generation: generation_from_i64(row.issuance_generation)?,
            status: parse_status(&row.status)?,
            issued_at: row.issued_at,
            expires_at: row.expires_at,
            acknowledged_at: row.acknowledged_at,
            revoked_at: row.revoked_at,
        })
    }
}

pub(super) fn generation_from_i64(
    value: i64,
) -> Result<RuntimeCredentialGeneration, RuntimeAuthorityError> {
    u64::try_from(value)
        .map_err(storage)
        .and_then(|value| RuntimeCredentialGeneration::new(value).map_err(storage))
}

pub(super) fn parse_status(value: &str) -> Result<RuntimeSessionStatus, RuntimeAuthorityError> {
    match value {
        "pending_handoff" => Ok(RuntimeSessionStatus::PendingHandoff),
        "active" => Ok(RuntimeSessionStatus::Active),
        "revoked" => Ok(RuntimeSessionStatus::Revoked),
        "expired" => Ok(RuntimeSessionStatus::Expired),
        _ => Err(RuntimeAuthorityError::Persistence),
    }
}

pub(super) fn storage<T>(_: T) -> RuntimeAuthorityError {
    RuntimeAuthorityError::Persistence
}

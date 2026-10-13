//! Rebuild only the immutable copied ceiling, not a current binding selection.

use super::super::{
    common::storage,
    gateway_helpers::{SnapshotBindingRow, stored_binding},
};
use super::{original::Original, session::Session};
use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, WorkloadKind, WorkloadPrincipal,
};
use runtime_authority::RuntimeAuthorityError;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Snapshot {
    id: Uuid,
    run_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    model: String,
    normalized_hash: Vec<u8>,
}
#[derive(sqlx::FromRow)]
struct CopiedBinding {
    ordinal: i32,
    copy_exact: bool,
    binding_id: Uuid,
    requirement_id: Uuid,
    slot_key: String,
    resource_kind: String,
    required_operations: Vec<String>,
    optional_operations: Vec<String>,
    slot_required: bool,
    requirement_hash: Vec<u8>,
    resource_id: Uuid,
    granted_operations: Vec<String>,
    binding_hash: Vec<u8>,
    authorization_model_version: String,
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    original: &Original,
    session: &Session,
) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
    let header = load_header(tx, original, session).await?;
    let rows = load_copied(tx, original, header.id).await?;
    decode(header, original, rows)
}

async fn load_header(
    tx: &mut Transaction<'_, Postgres>,
    original: &Original,
    session: &Session,
) -> Result<Snapshot, RuntimeAuthorityError> {
    let mut headers: Vec<Snapshot> = sqlx::query_as(
        "SELECT id,run_id,instance_id,instance_revision_id AS revision_id,
                authorization_model_version AS model,normalized_hash
         FROM public.run_authorization_snapshots WHERE id=$1 OR run_id=$2 LIMIT 2",
    )
    .bind(session.snapshot_id)
    .bind(original.id)
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    if headers.len() != 1 {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    let header = headers.pop().ok_or(RuntimeAuthorityError::NotFound)?;
    if header.id != session.snapshot_id
        || header.run_id != original.id
        || header.instance_id != original.instance_id
        || header.revision_id != original.revision_id
        || header.normalized_hash != session.snapshot_hash
    {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    Ok(header)
}

async fn load_copied(
    tx: &mut Transaction<'_, Postgres>,
    original: &Original,
    snapshot: Uuid,
) -> Result<Vec<CopiedBinding>, RuntimeAuthorityError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.run_authorization_snapshot_bindings WHERE snapshot_id=$1",
    )
    .bind(snapshot)
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)?;
    // All rows are selected by the copied snapshot first. Complete count equality
    // denies missing join endpoints rather than silently dropping orphan bindings.
    let rows: Vec<CopiedBinding> = sqlx::query_as(
        "SELECT copied.ordinal,
                (ROW(copied.instance_revision_id,copied.binding_hash,copied.slot_key,
                     copied.resource_kind,copied.resource_id,copied.granted_operations)
                 IS NOT DISTINCT FROM ROW(binding.instance_revision_id,binding.normalized_hash,
                     binding.slot_key,binding.resource_kind,binding.resource_id,binding.granted_operations)
                 AND binding.requirement_hash=requirement.normalized_hash
                 AND binding.slot_key=requirement.slot_key
                 AND binding.resource_kind=requirement.resource_kind) AS copy_exact,
                binding.id AS binding_id,requirement.id AS requirement_id,
                requirement.slot_key,requirement.resource_kind,requirement.required_operations,
                requirement.optional_operations,requirement.slot_required,
                requirement.normalized_hash AS requirement_hash,binding.resource_id,
                binding.granted_operations,binding.normalized_hash AS binding_hash,
                binding.authorization_model_version
         FROM public.run_authorization_snapshot_bindings copied
         JOIN public.agent_capability_bindings binding ON binding.id=copied.binding_id
           AND binding.instance_revision_id=copied.instance_revision_id
         JOIN public.release_capability_requirements requirement ON requirement.id=binding.requirement_id
           AND requirement.release_agent_id=binding.release_agent_id
         WHERE copied.snapshot_id=$1 AND binding.instance_revision_id=$2
           AND binding.release_agent_id=$3 ORDER BY copied.ordinal")
        .bind(snapshot).bind(original.revision_id).bind(original.release_agent_id)
        .fetch_all(&mut **tx).await.map_err(storage)?;
    if i64::try_from(rows.len()).map_err(storage)? != count {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    Ok(rows)
}

fn decode(
    header: Snapshot,
    original: &Original,
    rows: Vec<CopiedBinding>,
) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
    let mut bindings = Vec::with_capacity(rows.len());
    let mut ordered_ids = Vec::with_capacity(rows.len());
    for (ordinal, row) in rows.into_iter().enumerate() {
        if !row.copy_exact || row.ordinal != i32::try_from(ordinal).map_err(storage)? {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        ordered_ids.push(row.binding_id);
        bindings.push(stored_binding(
            SnapshotBindingRow {
                binding_id: row.binding_id,
                requirement_id: row.requirement_id,
                slot_key: row.slot_key,
                resource_kind: row.resource_kind,
                required_operations: row.required_operations,
                optional_operations: row.optional_operations,
                slot_required: row.slot_required,
                requirement_hash: row.requirement_hash,
                resource_id: row.resource_id,
                granted_operations: row.granted_operations,
                binding_hash: row.binding_hash,
                authorization_model_version: row.authorization_model_version,
            },
            &header.model,
        )?);
    }
    let snapshot = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(header.id),
        WorkloadPrincipal::new(
            WorkloadKind::AgentInstance,
            original.instance_id,
            original.revision_id,
        ),
        header.model,
        bindings,
    )
    .map_err(storage)?;
    if snapshot.normalized_hash().as_bytes().as_slice() != header.normalized_hash.as_slice()
        || !snapshot
            .bindings()
            .map(|binding| binding.id().as_uuid())
            .eq(ordered_ids)
    {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    Ok(snapshot)
}

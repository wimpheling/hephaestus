//! Recheck the generic immutable rows in the independently held Git snapshot.

use crate::storage;
use runtime_authority::ExpiredRuntimeHandoff;
use runtime_git_authority::RuntimeGitAuthorityError;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct OriginalRow {
    session: Uuid,
    snapshot: Uuid,
    run: Uuid,
    instance: Uuid,
    revision: Uuid,
    release: Uuid,
    agent: Uuid,
    command: Uuid,
    attachment: Option<Uuid>,
    identity_hash: Vec<u8>,
    snapshot_hash: Vec<u8>,
    credential_hash: Vec<u8>,
    generation: i64,
    status: String,
    issued: OffsetDateTime,
    expires: OffsetDateTime,
    acknowledged: Option<OffsetDateTime>,
    revoked: Option<OffsetDateTime>,
    revocation_reason: Option<String>,
    model: String,
}

pub(super) async fn original(
    tx: &mut Transaction<'_, Postgres>,
    original: &ExpiredRuntimeHandoff,
) -> Result<(), RuntimeGitAuthorityError> {
    let rows: Vec<OriginalRow> = sqlx::query_as(
        "SELECT session.id AS session,session.snapshot_id AS snapshot,execution.id AS run,
                execution.instance_id AS instance,execution.instance_revision_id AS revision,
                execution.release_id AS release,execution.release_agent_id AS agent,
                execution.command_id AS command,execution.attachment_id AS attachment,
                session.identity_hash,session.snapshot_hash,session.credential_hash,
                session.issuance_generation AS generation,session.status,session.issued_at AS issued,
                session.expires_at AS expires,session.acknowledged_at AS acknowledged,
                session.revoked_at AS revoked,session.revocation_reason,
                snapshot.authorization_model_version AS model
         FROM public.runtime_authority_sessions session
         JOIN public.runs execution ON execution.id=session.run_id
           AND execution.instance_id=session.instance_id
           AND execution.instance_revision_id=session.instance_revision_id
           AND execution.attachment_id IS NOT DISTINCT FROM session.attachment_id
         JOIN public.run_authorization_snapshots snapshot ON snapshot.id=session.snapshot_id
           AND snapshot.run_id=execution.id AND snapshot.instance_id=execution.instance_id
           AND snapshot.instance_revision_id=execution.instance_revision_id
           AND snapshot.normalized_hash=session.snapshot_hash
         WHERE session.id=$1 OR session.run_id=$2 LIMIT 2")
        .bind(original.session().id.as_uuid()).bind(original.pins().run())
        .fetch_all(&mut **tx).await.map_err(storage)?;
    let [row] = rows.as_slice() else {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    };
    row.matches(original)?;
    let associations: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM public.runtime_authority_sessions WHERE id=$1 OR run_id=$2),
                (SELECT count(*) FROM public.run_authorization_snapshots WHERE id=$3 OR run_id=$2)",
    )
    .bind(original.session().id.as_uuid())
    .bind(original.pins().run())
    .bind(original.snapshot().id().as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)?;
    if associations != (1, 1) {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    }
    bindings(tx, original).await
}

impl OriginalRow {
    fn matches(&self, original: &ExpiredRuntimeHandoff) -> Result<(), RuntimeGitAuthorityError> {
        let session = original.session();
        let pins = original.pins();
        if self.session != session.id.as_uuid()
            || self.snapshot != session.snapshot_id.as_uuid()
            || self.run != pins.run()
            || self.instance != pins.instance()
            || self.revision != pins.revision()
            || self.release != pins.release()
            || self.agent != pins.release_agent()
            || self.command != pins.command()
            || self.attachment != original.attachment()
            || self.identity_hash.as_slice() != session.identity_hash.as_bytes()
            || self.snapshot_hash.as_slice() != original.snapshot().normalized_hash().as_bytes()
            || self.credential_hash.as_slice() != original.credential_hash().as_bytes()
            || u64::try_from(self.generation).ok() != Some(session.generation.get())
            || self.status != "expired"
            || self.issued != session.issued_at
            || self.expires != session.expires_at
            || self.acknowledged != session.acknowledged_at
            || self.revoked != session.revoked_at
            || self.revocation_reason.is_some()
            || self.model != original.snapshot().authorization_model_version()
        {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct BindingRow {
    revision: Uuid,
    ordinal: i32,
    id: Uuid,
    hash: Vec<u8>,
    slot: String,
    kind: String,
    resource: Uuid,
    operations: Vec<String>,
}

async fn bindings(
    tx: &mut Transaction<'_, Postgres>,
    original: &ExpiredRuntimeHandoff,
) -> Result<(), RuntimeGitAuthorityError> {
    let rows: Vec<BindingRow> = sqlx::query_as(
        "SELECT instance_revision_id AS revision,ordinal,binding_id AS id,binding_hash AS hash,
                slot_key AS slot,resource_kind AS kind,resource_id AS resource,granted_operations AS operations
         FROM public.run_authorization_snapshot_bindings WHERE snapshot_id=$1 ORDER BY ordinal")
        .bind(original.snapshot().id().as_uuid()).fetch_all(&mut **tx).await.map_err(storage)?;
    if rows.len() != original.snapshot().bindings().len() {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    }
    for ((ordinal, row), binding) in rows
        .into_iter()
        .enumerate()
        .zip(original.snapshot().bindings())
    {
        let expected_kind = binding.resource().kind.as_str();
        if row.revision != original.pins().revision()
            || row.ordinal != i32::try_from(ordinal).map_err(storage)?
            || row.id != binding.id().as_uuid()
            || row.hash.as_slice() != binding.normalized_hash().as_bytes()
            || row.slot != binding.slot().as_str()
            || row.kind != expected_kind
            || row.resource != binding.resource().id
            || !row.operations.iter().map(String::as_str).eq(binding
                .granted_operations()
                .map(capability_domain::CapabilityOperation::as_str))
        {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
    }
    Ok(())
}

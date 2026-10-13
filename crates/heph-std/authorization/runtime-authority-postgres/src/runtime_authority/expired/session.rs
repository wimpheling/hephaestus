//! Complete unique session and snapshot links; raw history is never rewritten.

use super::{
    super::common::{SessionRow, storage},
    original::Original,
};
use capability_domain::{
    AuthorizationSnapshot, RuntimeCredentialGeneration, RuntimeCredentialHash, RuntimeInvocation,
    RuntimeSessionId, RuntimeSessionIdentity,
};
use runtime_authority::{ExpiredRuntimeHandoff, RuntimeAuthorityError, StoredRuntimeSession};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub(super) struct Session {
    id: Uuid,
    pub(super) snapshot_id: Uuid,
    run_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment: Option<Uuid>,
    identity_hash: Vec<u8>,
    pub(super) snapshot_hash: Vec<u8>,
    credential_hash: Vec<u8>,
    issuance_generation: i64,
    status: String,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    acknowledged_at: Option<OffsetDateTime>,
    revoked_at: Option<OffsetDateTime>,
    revocation_reason: Option<String>,
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    original: &Original,
    id: RuntimeSessionId,
    generation: RuntimeCredentialGeneration,
) -> Result<Session, RuntimeAuthorityError> {
    // OR exposes a collision/orphan rather than filtering it out by expected owner.
    let mut rows: Vec<Session> = sqlx::query_as(
        "SELECT id,snapshot_id,run_id,instance_id,instance_revision_id AS revision_id,
                attachment_id AS attachment,identity_hash,snapshot_hash,credential_hash,
                issuance_generation,status,issued_at,expires_at,acknowledged_at,revoked_at,
                revocation_reason FROM public.runtime_authority_sessions
         WHERE id=$1 OR run_id=$2 LIMIT 2",
    )
    .bind(id.as_uuid())
    .bind(original.id)
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    if rows.len() != 1 {
        return Err(RuntimeAuthorityError::NotFound);
    }
    let row = rows.pop().ok_or(RuntimeAuthorityError::NotFound)?;
    row.check(original, id, generation)?;
    Ok(row)
}

impl Session {
    fn check(
        &self,
        original: &Original,
        id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeAuthorityError> {
        if self.id != id.as_uuid()
            || self.run_id != original.id
            || self.instance_id != original.instance_id
            || self.revision_id != original.revision_id
            || self.attachment != original.attachment
            || self.snapshot_id.is_nil()
            || self.status != "expired"
            || self.revoked_at.is_some()
            || self.revocation_reason.is_some()
            || self.credential_hash.len() != 32
            || self.identity_hash.len() != 32
            || self.snapshot_hash.len() != 32
        {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        if super::super::common::generation_from_i64(self.issuance_generation)? != generation {
            return Err(RuntimeAuthorityError::GenerationMismatch);
        }
        Ok(())
    }
    pub(super) fn project(
        self,
        original: &Original,
        snapshot: AuthorizationSnapshot,
        observed: OffsetDateTime,
    ) -> Result<ExpiredRuntimeHandoff, RuntimeAuthorityError> {
        if snapshot.normalized_hash().as_bytes().as_slice() != self.snapshot_hash.as_slice() {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        let identity = RuntimeSessionIdentity::new(
            RuntimeSessionId::from_uuid(self.id),
            snapshot.principal(),
            RuntimeInvocation::Run(original.id.into()),
            &snapshot,
            self.issued_at,
            self.expires_at,
        )
        .map_err(storage)?;
        let verifier =
            RuntimeCredentialHash::from_bytes(self.credential_hash.try_into().map_err(storage)?);
        let stored = StoredRuntimeSession::try_from(SessionRow {
            id: self.id,
            snapshot_id: self.snapshot_id,
            identity_hash: self.identity_hash,
            issuance_generation: self.issuance_generation,
            status: self.status,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
            acknowledged_at: self.acknowledged_at,
            revoked_at: self.revoked_at,
        })?;
        // Unrecoverable nanosecond claims from old ordinary issuance fail this
        // exact hash check; never round/rewrite history or infer original times.
        ExpiredRuntimeHandoff::new(
            stored,
            snapshot,
            identity,
            original.pins()?,
            self.attachment,
            verifier,
            observed,
        )
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

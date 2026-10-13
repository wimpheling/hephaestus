use capability_domain::AuthorityHash;
use runtime_authority::{
    LiveRunSessionCeiling, LiveRunSnapshotPins, RuntimeAuthorityError, StoredRuntimeSession,
};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;

use super::super::common::{SessionRow, storage};

#[derive(sqlx::FromRow)]
struct IssuedSession {
    id: uuid::Uuid,
    snapshot_id: uuid::Uuid,
    identity_hash: Vec<u8>,
    snapshot_hash: Vec<u8>,
    issuance_generation: i64,
    status: String,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    acknowledged_at: Option<OffsetDateTime>,
    revoked_at: Option<OffsetDateTime>,
    ceiling_hash: Vec<u8>,
}

impl IssuedSession {
    fn decode(self) -> Result<(StoredRuntimeSession, AuthorityHash), RuntimeAuthorityError> {
        let session_hash = hash(&self.snapshot_hash)?;
        if hash(&self.ceiling_hash)? != session_hash {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
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
        Ok((stored, session_hash))
    }
}

async fn load(
    tx: &mut Transaction<'_, Postgres>,
    pins: LiveRunSnapshotPins,
    require_live: bool,
) -> Result<Option<IssuedSession>, RuntimeAuthorityError> {
    sqlx::query_as(
        "SELECT session.id,session.snapshot_id,session.identity_hash,session.snapshot_hash,
                session.issuance_generation,session.status,session.issued_at,session.expires_at,
                session.acknowledged_at,session.revoked_at,snapshot.normalized_hash AS ceiling_hash
         FROM runtime_authority_sessions session
         JOIN run_authorization_snapshots snapshot ON snapshot.id=session.snapshot_id
           AND snapshot.run_id=session.run_id AND snapshot.instance_id=session.instance_id
           AND snapshot.instance_revision_id=session.instance_revision_id
         JOIN runs run ON run.id=session.run_id
         WHERE session.id=$1 AND session.run_id=$1 AND session.instance_id=$2
           AND session.instance_revision_id=$3 AND session.snapshot_id=$1
           AND session.attachment_id IS NOT DISTINCT FROM run.attachment_id
           AND (NOT $4 OR (session.status IN ('pending_handoff','active') AND session.revoked_at IS NULL
             AND session.issued_at<=clock_timestamp() AND session.expires_at>clock_timestamp()))",
    )
    .bind(pins.run()).bind(pins.instance()).bind(pins.revision()).bind(require_live)
    .fetch_optional(&mut **tx).await.map_err(storage)
}

pub async fn validate(
    tx: &mut Transaction<'_, Postgres>,
    pins: LiveRunSnapshotPins,
    expected: LiveRunSessionCeiling,
    fresh_hash: AuthorityHash,
    now: OffsetDateTime,
) -> Result<(), RuntimeAuthorityError> {
    let row = load(tx, pins, true)
        .await?
        .ok_or(RuntimeAuthorityError::NotFound)?;
    let (stored, session_hash) = row.decode()?;
    if session_hash != fresh_hash || !expected.matches(&stored, session_hash, now) {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    Ok(())
}

pub async fn recorded(
    tx: &mut Transaction<'_, Postgres>,
    pins: LiveRunSnapshotPins,
) -> Result<Option<LiveRunSessionCeiling>, RuntimeAuthorityError> {
    let Some(row) = load(tx, pins, false).await? else {
        // An orphan or contradictory record must not become permission to
        // recreate a session. None means both durable records are truly absent.
        let evidence: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE id=$1 OR run_id=$1)
                OR EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE id=$1 OR run_id=$1)",
        )
        .bind(pins.run())
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
        return if evidence {
            Err(RuntimeAuthorityError::IdentityMismatch)
        } else {
            Ok(None)
        };
    };
    let (stored, session_hash) = row.decode()?;
    let ceiling = LiveRunSessionCeiling::new(&stored, session_hash)?;
    let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    if !ceiling.matches(&stored, session_hash, now) {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    Ok(Some(ceiling))
}

fn hash(value: &[u8]) -> Result<AuthorityHash, RuntimeAuthorityError> {
    value
        .try_into()
        .map(AuthorityHash::from_bytes)
        .map_err(|_| RuntimeAuthorityError::Persistence)
}

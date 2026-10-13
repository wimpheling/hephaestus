//! Protected complete graph creation and pending handoff revalidation.

use super::super::{
    agent_repository::PgRuntimeSessionRepository,
    common::{SessionRow, storage},
};
use capability_domain::{
    RuntimeCredentialGeneration, RuntimeInvocation, RuntimeSessionId, WorkloadKind,
};
use runtime_authority::{NewRuntimeSession, RuntimeAuthorityError, StoredRuntimeSession};
use time::OffsetDateTime;

impl PgRuntimeSessionRepository {
    pub(in crate::runtime_authority) async fn qualified_invocation_session(
        &self,
        session: RuntimeSessionId,
    ) -> Result<bool, RuntimeAuthorityError> {
        let run: Option<uuid::Uuid> =
            sqlx::query_scalar("SELECT run_id FROM runtime_authority_sessions WHERE id=$1")
                .bind(session.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(storage)?;
        self.qualified_invocation_run(run.unwrap_or(session.as_uuid()))
            .await
    }

    pub(in crate::runtime_authority) async fn invocation_find(
        &self,
        session: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError> {
        sqlx::query_as::<_,SessionRow>(
            "SELECT id,snapshot_id,identity_hash,issuance_generation,status,issued_at,expires_at,acknowledged_at,revoked_at FROM find_invocation119_session($1,$2)",
        ).bind(session.as_uuid()).bind(self.invocation_config_bytes()?)
            .fetch_optional(&self.pool).await.map_err(storage)?
            .map(TryInto::try_into).transpose()
    }

    pub(in crate::runtime_authority) async fn invocation_create(
        &self,
        session: NewRuntimeSession<'_>,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let (WorkloadKind::AgentInstance, RuntimeInvocation::Run(run)) = (
            session.identity.principal().kind,
            session.identity.invocation(),
        ) else {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        };
        let config = self.invocation_config_bytes()?;
        super::configuration::interval(
            session.identity.issued_at(),
            session.identity.expires_at(),
        )?;
        if session.identity.id().as_uuid() != run.as_uuid()
            || session.snapshot.id().as_uuid() != run.as_uuid()
            || session.identity.principal() != session.snapshot.principal()
            || session.attachment_id.is_some()
        {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        let mut tx = self.pool.begin().await.map_err(storage)?;
        super::reservations::acquire(&mut tx, run.as_uuid()).await?;
        let row: SessionRow=sqlx::query_as(
            "SELECT id,snapshot_id,identity_hash,issuance_generation,status,issued_at,expires_at,acknowledged_at,revoked_at FROM write_invocation119_session($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        ).bind(run.as_uuid()).bind(config).bind(session.snapshot.authorization_model_version())
            .bind(session.snapshot.normalized_hash().as_bytes().as_slice())
            .bind(session.identity.normalized_hash().as_bytes().as_slice())
            .bind(i64::try_from(session.generation.get()).map_err(storage)?)
            .bind(session.credential_hash.as_bytes().as_slice())
            .bind(session.identity.issued_at()).bind(session.identity.expires_at())
            .bind(session.identity.issued_at().unix_timestamp_nanos().to_string())
            .bind(session.identity.expires_at().unix_timestamp_nanos().to_string())
            .fetch_one(&mut *tx).await.map_err(storage)?;
        let stored = StoredRuntimeSession::try_from(row)?;
        if stored.identity_hash != session.identity.normalized_hash()
            || stored.snapshot_id != session.snapshot.id()
            || stored.generation != session.generation
        {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        tx.commit().await.map_err(storage)?;
        Ok(stored)
    }

    pub(in crate::runtime_authority) async fn invocation_acknowledge(
        &self,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        at: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let config = self.invocation_config_bytes()?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        super::reservations::acquire(&mut tx, session.as_uuid()).await?;
        let row:SessionRow=sqlx::query_as(
            "SELECT id,snapshot_id,identity_hash,issuance_generation,status,issued_at,expires_at,acknowledged_at,revoked_at FROM acknowledge_invocation119_session($1,$2,$3,$4)",
        ).bind(session.as_uuid()).bind(config)
            .bind(i64::try_from(generation.get()).map_err(storage)?).bind(at)
            .fetch_one(&mut *tx).await.map_err(storage)?;
        let stored = StoredRuntimeSession::try_from(row)?;
        tx.commit().await.map_err(storage)?;
        Ok(stored)
    }
}

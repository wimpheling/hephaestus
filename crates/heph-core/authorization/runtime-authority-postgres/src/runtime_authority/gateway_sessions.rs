use super::{
    EXPIRY_BATCH_SIZE, HTTP_HANDLER_CONTRACT_V1,
    common::{SessionRow, generation_from_i64, parse_status, storage},
    gateway_helpers::insert_gateway_snapshot_bindings,
    gateway_repository::PgGatewayRuntimeSessionRepository,
};
use async_trait::async_trait;
use capability_domain::{
    RuntimeCredentialGeneration, RuntimeInvocation, RuntimeSessionId, RuntimeSessionStatus,
    WorkloadKind,
};
use runtime_authority::{
    NewRuntimeSession, RuntimeAuthorityError, RuntimeSessionRepository, StoredRuntimeSession,
};
use time::OffsetDateTime;
use uuid::Uuid;

#[async_trait]
impl RuntimeSessionRepository for PgGatewayRuntimeSessionRepository {
    async fn find(
        &self,
        session_id: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM gateway_runtime_authority_sessions WHERE id = $1",
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
        session: NewRuntimeSession<'_>,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let principal = session.identity.principal();
        let (WorkloadKind::Gateway, RuntimeInvocation::Gateway(invocation_id)) =
            (principal.kind, session.identity.invocation())
        else {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        };
        let handler_contract = sqlx::query_scalar::<_, String>(
            "SELECT handler_contract FROM gateway_revisions
             WHERE id = $1 AND gateway_id = $2",
        )
        .bind(principal.revision_id)
        .bind(principal.id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        if handler_contract.as_deref() != Some(HTTP_HANDLER_CONTRACT_V1) {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        sqlx::query(
            "INSERT INTO gateway_authorization_snapshots
                (id, invocation_id, gateway_id, gateway_revision_id,
                 authorization_model_version, normalized_hash)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session.snapshot.id().as_uuid())
        .bind(invocation_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(session.snapshot.authorization_model_version())
        .bind(session.snapshot.normalized_hash().as_bytes().as_slice())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        insert_gateway_snapshot_bindings(&mut transaction, session.snapshot).await?;
        let generation = i64::try_from(session.generation.get()).map_err(storage)?;
        sqlx::query(
            "INSERT INTO gateway_runtime_authority_sessions
                (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
                 identity_hash, snapshot_hash, issuance_generation, credential_hash,
                 admission_mode, status, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     'guest_handoff', 'pending_handoff', $10, $11)",
        )
        .bind(session.identity.id().as_uuid())
        .bind(session.snapshot.id().as_uuid())
        .bind(invocation_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(session.identity.normalized_hash().as_bytes().as_slice())
        .bind(session.snapshot.normalized_hash().as_bytes().as_slice())
        .bind(generation)
        .bind(session.credential_hash.as_bytes().as_slice())
        .bind(session.identity.issued_at())
        .bind(session.identity.expires_at())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(StoredRuntimeSession {
            id: session.identity.id(),
            snapshot_id: session.snapshot.id(),
            identity_hash: session.identity.normalized_hash(),
            generation: session.generation,
            status: RuntimeSessionStatus::PendingHandoff,
            issued_at: session.identity.issued_at(),
            expires_at: session.identity.expires_at(),
            acknowledged_at: None,
            revoked_at: None,
        })
    }

    async fn acknowledge(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        acknowledged_at: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let current = Self::locked(&mut transaction, session_id).await?;
        let admission_mode = sqlx::query_scalar::<_, String>(
            "SELECT admission_mode FROM gateway_runtime_authority_sessions WHERE id = $1",
        )
        .bind(session_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage)?;
        if admission_mode != "guest_handoff" {
            return Err(RuntimeAuthorityError::SessionNotPending);
        }
        if generation_from_i64(current.issuance_generation)? != generation {
            return Err(RuntimeAuthorityError::GenerationMismatch);
        }
        match parse_status(&current.status)? {
            RuntimeSessionStatus::PendingHandoff
                if acknowledged_at >= current.issued_at && acknowledged_at < current.expires_at =>
            {
                sqlx::query("UPDATE gateway_runtime_authority_sessions SET status = 'active', acknowledged_at = $2, updated_at = $2 WHERE id = $1")
                    .bind(session_id.as_uuid()).bind(acknowledged_at).execute(&mut *transaction).await.map_err(storage)?;
            }
            RuntimeSessionStatus::Active => {}
            RuntimeSessionStatus::PendingHandoff
            | RuntimeSessionStatus::Revoked
            | RuntimeSessionStatus::Expired => {
                return Err(RuntimeAuthorityError::SessionNotPending);
            }
        }
        transaction.commit().await.map_err(storage)?;
        self.find(session_id)
            .await?
            .ok_or(RuntimeAuthorityError::Persistence)
    }

    async fn revoke(
        &self,
        session_id: RuntimeSessionId,
        revoked_at: OffsetDateTime,
        reason: &str,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        if reason.is_empty() || reason.len() > 256 {
            return Err(RuntimeAuthorityError::Persistence);
        }
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let invocation_id: Uuid = sqlx::query_scalar(
            "SELECT invocation_id FROM gateway_runtime_authority_sessions WHERE id = $1",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(RuntimeAuthorityError::NotFound)?;
        sqlx::query("SELECT id FROM gateway_invocations WHERE id = $1 FOR UPDATE")
            .bind(invocation_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage)?;
        let current = Self::locked(&mut transaction, session_id).await?;
        let admission_mode = sqlx::query_scalar::<_, String>(
            "SELECT admission_mode FROM gateway_runtime_authority_sessions WHERE id = $1",
        )
        .bind(session_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage)?;
        if revoked_at < current.issued_at {
            return Err(RuntimeAuthorityError::Persistence);
        }
        match parse_status(&current.status)? {
            RuntimeSessionStatus::PendingHandoff | RuntimeSessionStatus::Active => {
                sqlx::query("UPDATE gateway_runtime_authority_sessions SET status = 'revoked', revoked_at = $2, revocation_reason = $3, updated_at = $2 WHERE id = $1")
                    .bind(session_id.as_uuid()).bind(revoked_at).bind(reason).execute(&mut *transaction).await.map_err(storage)?;
            }
            RuntimeSessionStatus::Revoked => {}
            RuntimeSessionStatus::Expired => return Err(RuntimeAuthorityError::SessionNotPending),
        }
        if admission_mode == "host_mediated" {
            sqlx::query(
                "UPDATE gateway_secret_leases
                    SET status = 'revoked', revoked_at = $2
                  WHERE runtime_session_id = $1 AND status = 'active'",
            )
            .bind(session_id.as_uuid())
            .bind(revoked_at)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        }
        transaction.commit().await.map_err(storage)?;
        self.find(session_id)
            .await?
            .ok_or(RuntimeAuthorityError::Persistence)
    }

    async fn expire(&self, now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        let mut expired = 0;
        loop {
            let mut transaction = self.pool.begin().await.map_err(storage)?;
            // Select identifiers without taking session locks. Each row is
            // subsequently locked in invocation -> session order, matching
            // issuance, terminal completion, and explicit revocation.
            let sessions: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT session.id, session.invocation_id
                   FROM gateway_runtime_authority_sessions AS session
                  WHERE session.status IN ('pending_handoff', 'active')
                    AND session.expires_at <= $1
                  ORDER BY session.invocation_id, session.id
                  LIMIT $2",
            )
            .bind(now)
            .bind(EXPIRY_BATCH_SIZE)
            .fetch_all(&mut *transaction)
            .await
            .map_err(storage)?;
            if sessions.is_empty() {
                transaction.commit().await.map_err(storage)?;
                break;
            }
            for (session_id, invocation_id) in sessions {
                sqlx::query("SELECT id FROM gateway_invocations WHERE id = $1 FOR UPDATE")
                    .bind(invocation_id)
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(storage)?;
                let admission_mode = sqlx::query_scalar::<_, String>(
                    "SELECT admission_mode FROM gateway_runtime_authority_sessions
                      WHERE id = $1 AND status IN ('pending_handoff', 'active')
                        AND expires_at <= $2 FOR UPDATE",
                )
                .bind(session_id)
                .bind(now)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(storage)?;
                let Some(admission_mode) = admission_mode else {
                    continue;
                };
                sqlx::query(
                    "UPDATE gateway_runtime_authority_sessions
                        SET status = 'expired', updated_at = $2
                      WHERE id = $1",
                )
                .bind(session_id)
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
                if admission_mode == "host_mediated" {
                    sqlx::query(
                        "UPDATE gateway_secret_leases
                            SET status = 'expired', revoked_at = $2
                          WHERE runtime_session_id = $1 AND status = 'active'",
                    )
                    .bind(session_id)
                    .bind(now)
                    .execute(&mut *transaction)
                    .await
                    .map_err(storage)?;
                }
                expired += 1;
            }
            transaction.commit().await.map_err(storage)?;
        }
        Ok(expired)
    }
}

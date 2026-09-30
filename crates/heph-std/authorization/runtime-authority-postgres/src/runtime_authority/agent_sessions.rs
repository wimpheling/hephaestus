use super::{
    agent_repository::PgRuntimeSessionRepository,
    common::{SessionRow, generation_from_i64, insert_runtime_git_snapshot, parse_status, storage},
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

#[async_trait]
impl RuntimeSessionRepository for PgRuntimeSessionRepository {
    async fn find(
        &self,
        session_id: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM runtime_authority_sessions WHERE id = $1",
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
        let (WorkloadKind::AgentInstance, RuntimeInvocation::Run(run_id)) =
            (principal.kind, session.identity.invocation())
        else {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        };
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        sqlx::query(
            "INSERT INTO run_authorization_snapshots
                (id, run_id, instance_id, instance_revision_id,
                 authorization_model_version, normalized_hash)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session.snapshot.id().as_uuid())
        .bind(run_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(session.snapshot.authorization_model_version())
        .bind(session.snapshot.normalized_hash().as_bytes().as_slice())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;

        for (ordinal, binding) in session.snapshot.bindings().enumerate() {
            let ordinal = i32::try_from(ordinal).map_err(storage)?;
            let operations = binding
                .granted_operations()
                .map(capability_domain::CapabilityOperation::as_str)
                .collect::<Vec<_>>();
            sqlx::query(
                "INSERT INTO run_authorization_snapshot_bindings
                    (snapshot_id, instance_revision_id, ordinal, binding_id,
                     binding_hash, slot_key, resource_kind, resource_id,
                     granted_operations)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(session.snapshot.id().as_uuid())
            .bind(principal.revision_id)
            .bind(ordinal)
            .bind(binding.id().as_uuid())
            .bind(binding.normalized_hash().as_bytes().as_slice())
            .bind(binding.slot().as_str())
            .bind(binding.resource().kind.as_str())
            .bind(binding.resource().id)
            .bind(&operations)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        }

        insert_runtime_git_snapshot(
            &mut transaction,
            session.snapshot.id().as_uuid(),
            principal.revision_id,
            run_id.as_uuid(),
        )
        .await?;

        let generation = i64::try_from(session.generation.get()).map_err(storage)?;
        sqlx::query(
            "INSERT INTO runtime_authority_sessions
                (id, snapshot_id, run_id, instance_id, instance_revision_id,
                 attachment_id, identity_hash, snapshot_hash,
                 issuance_generation, credential_hash, status, issued_at,
                 expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     'pending_handoff', $11, $12)",
        )
        .bind(session.identity.id().as_uuid())
        .bind(session.snapshot.id().as_uuid())
        .bind(run_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(session.attachment_id)
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
        let current_generation = generation_from_i64(current.issuance_generation)?;
        if current_generation != generation {
            return Err(RuntimeAuthorityError::GenerationMismatch);
        }
        match parse_status(&current.status)? {
            RuntimeSessionStatus::PendingHandoff
                if acknowledged_at >= current.issued_at && acknowledged_at < current.expires_at =>
            {
                sqlx::query(
                    "UPDATE runtime_authority_sessions
                     SET status = 'active', acknowledged_at = $2,
                         updated_at = $2
                     WHERE id = $1",
                )
                .bind(session_id.as_uuid())
                .bind(acknowledged_at)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
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
        let current = Self::locked(&mut transaction, session_id).await?;
        if revoked_at < current.issued_at {
            return Err(RuntimeAuthorityError::Persistence);
        }
        match parse_status(&current.status)? {
            RuntimeSessionStatus::PendingHandoff | RuntimeSessionStatus::Active => {
                sqlx::query(
                    "UPDATE runtime_authority_sessions
                     SET status = 'revoked', revoked_at = $2,
                         revocation_reason = $3, updated_at = $2
                     WHERE id = $1",
                )
                .bind(session_id.as_uuid())
                .bind(revoked_at)
                .bind(reason)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
            }
            RuntimeSessionStatus::Revoked => {}
            RuntimeSessionStatus::Expired => {
                return Err(RuntimeAuthorityError::SessionNotPending);
            }
        }
        transaction.commit().await.map_err(storage)?;
        self.find(session_id)
            .await?
            .ok_or(RuntimeAuthorityError::Persistence)
    }

    async fn expire(&self, now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        sqlx::query(
            "UPDATE runtime_authority_sessions
             SET status = 'expired', updated_at = $1
             WHERE status IN ('pending_handoff', 'active')
               AND expires_at <= $1",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map(|result| result.rows_affected())
        .map_err(storage)
    }
}

use super::{
    common::{SessionRow, storage},
    gateway_helpers::insert_gateway_snapshot_bindings,
};
use capability_domain::{
    AuthorizationSnapshot, RuntimeCredentialGeneration, RuntimeInvocation, RuntimeSessionId,
    RuntimeSessionStatus, WorkloadKind,
};
use runtime_authority::{RuntimeAuthorityError, StoredRuntimeSession};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// `PostgreSQL` storage for gateway-only runtime snapshots and sessions.
///
/// This deliberately implements the same generic session repository contract
/// as agent runs while targeting the gateway-specific immutable tables.
#[derive(Clone)]
pub struct PgGatewayRuntimeSessionRepository {
    pub(super) pool: PgPool,
}

impl PgGatewayRuntimeSessionRepository {
    /// Creates a gateway session repository over a worker-role pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Persists an active host-mediated service session without creating a
    /// guest bearer or using the runtime handoff store.
    // Keep invocation locking, snapshot insertion, and session insertion in one
    // auditable transaction to preserve the issuance/completion lock order.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn create_host_mediated(
        &self,
        snapshot: &AuthorizationSnapshot,
        identity: &capability_domain::RuntimeSessionIdentity,
        generation: RuntimeCredentialGeneration,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let principal = identity.principal();
        let (WorkloadKind::Gateway, RuntimeInvocation::Gateway(invocation_id)) =
            (principal.kind, identity.invocation())
        else {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        };
        if snapshot.bindings().next().is_some() {
            return Err(RuntimeAuthorityError::Persistence);
        }
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let invocation: (Uuid, Uuid, String) = sqlx::query_as(
            "SELECT gateway_id, gateway_revision_id, outcome
               FROM gateway_invocations WHERE id = $1 FOR UPDATE",
        )
        .bind(invocation_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage)?;
        if invocation.0 != principal.id || invocation.1 != principal.revision_id {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        if invocation.2 != "accepted" {
            return Err(RuntimeAuthorityError::SessionNotPending);
        }
        if let Some(current) = sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM gateway_runtime_authority_sessions
             WHERE invocation_id = $1 FOR UPDATE",
        )
        .bind(invocation_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        {
            let mode = sqlx::query_scalar::<_, String>(
                "SELECT admission_mode FROM gateway_runtime_authority_sessions WHERE id = $1",
            )
            .bind(current.id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage)?;
            let stored: StoredRuntimeSession = current.try_into()?;
            if mode != "host_mediated"
                || stored.snapshot_id != snapshot.id()
                || stored.identity_hash != identity.normalized_hash()
            {
                return Err(RuntimeAuthorityError::IdentityMismatch);
            }
            if stored.status != RuntimeSessionStatus::Active || stored.acknowledged_at.is_some() {
                return Err(RuntimeAuthorityError::SessionNotPending);
            }
            transaction.commit().await.map_err(storage)?;
            return Ok(stored);
        }

        sqlx::query(
            "INSERT INTO gateway_authorization_snapshots
                (id, invocation_id, gateway_id, gateway_revision_id,
                 authorization_model_version, normalized_hash)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(snapshot.id().as_uuid())
        .bind(invocation_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(snapshot.authorization_model_version())
        .bind(snapshot.normalized_hash().as_bytes().as_slice())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        insert_gateway_snapshot_bindings(&mut transaction, snapshot).await?;
        let generation_value = i64::try_from(generation.get()).map_err(storage)?;
        sqlx::query(
            "INSERT INTO gateway_runtime_authority_sessions
                (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
                 identity_hash, snapshot_hash, issuance_generation, credential_hash,
                 admission_mode, status, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL,
                     'host_mediated', 'active', $9, $10)",
        )
        .bind(identity.id().as_uuid())
        .bind(snapshot.id().as_uuid())
        .bind(invocation_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(identity.normalized_hash().as_bytes().as_slice())
        .bind(snapshot.normalized_hash().as_bytes().as_slice())
        .bind(generation_value)
        .bind(identity.issued_at())
        .bind(identity.expires_at())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(StoredRuntimeSession {
            id: identity.id(),
            snapshot_id: snapshot.id(),
            identity_hash: identity.normalized_hash(),
            generation,
            status: RuntimeSessionStatus::Active,
            issued_at: identity.issued_at(),
            expires_at: identity.expires_at(),
            acknowledged_at: None,
            revoked_at: None,
        })
    }

    pub(super) async fn locked(
        transaction: &mut Transaction<'_, Postgres>,
        session_id: RuntimeSessionId,
    ) -> Result<SessionRow, RuntimeAuthorityError> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM gateway_runtime_authority_sessions WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?
        .ok_or(RuntimeAuthorityError::NotFound)
    }
}

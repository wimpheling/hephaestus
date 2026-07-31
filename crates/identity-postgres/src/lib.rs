//! `PostgreSQL` implementations of identity application ports.

use async_trait::async_trait;
use identity_application::{
    BootstrapIdentity, BootstrapIdentityError, IdempotentIdentityResolver, IdentityBootstrapper,
    IdentityMappingError, ResolveIdentityError, ResolveVerifiedIdentity, ResolvedIdentity,
    VerifiedExternalIdentity, VerifiedIdentityMapper,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId, actor_idempotency_id};
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

/// PostgreSQL-backed identity mapping and bootstrap adapter.
#[derive(Clone)]
pub struct PostgresIdentityStore {
    pool: PgPool,
}

impl PostgresIdentityStore {
    /// Creates an identity adapter backed by the supplied database pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl VerifiedIdentityMapper for PostgresIdentityStore {
    async fn map_verified_identity(
        &self,
        verified: &VerifiedExternalIdentity,
        request_id: RequestId,
        trace_id: Option<&str>,
    ) -> Result<AuthenticatedIdentity, IdentityMappingError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(IdentityMappingError::provider)?;
        let identity =
            map_verified_in_transaction(&mut transaction, verified, request_id, trace_id).await?;
        transaction
            .commit()
            .await
            .map_err(IdentityMappingError::provider)?;
        Ok(identity)
    }
}

#[async_trait]
impl IdempotentIdentityResolver for PostgresIdentityStore {
    async fn resolve_verified_identity(
        &self,
        request: ResolveVerifiedIdentity,
    ) -> Result<ResolvedIdentity, ResolveIdentityError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(ResolveIdentityError::provider)?;
        let mapped_user_id: Uuid = sqlx::query_scalar(
            "SELECT users.id
             FROM external_identities external
             JOIN users ON users.id = external.user_id
             WHERE external.issuer = $1 AND external.subject = $2
               AND users.status = 'active'",
        )
        .bind(&request.verified.issuer)
        .bind(&request.verified.subject)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ResolveIdentityError::provider)?
        .ok_or(ResolveIdentityError::PermissionDenied)?;
        let idempotency_id =
            actor_idempotency_id(mapped_user_id.as_bytes(), &request.idempotency_seed);
        let prior = sqlx::query_as::<_, PriorResolutionRow>(
            "SELECT users.id AS user_id, users.display_name, profile.validated_claims
             FROM external_identities external
             JOIN users ON users.id = external.user_id
             JOIN user_profiles profile ON profile.user_id = users.id
             JOIN application_events event
               ON event.occurrence_id = $3
              AND event.actor_id = users.id
              AND event.aggregate_type = 'identity_profile'
              AND event.scope_kind = 'identity'
              AND event.scope_id = users.id
             WHERE external.issuer = $1 AND external.subject = $2
             LIMIT 1",
        )
        .bind(&request.verified.issuer)
        .bind(&request.verified.subject)
        .bind(idempotency_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ResolveIdentityError::provider)?;
        if let Some(prior) = prior {
            if prior.validated_claims != request.verified.claims {
                return Err(ResolveIdentityError::IdempotencyConflict);
            }
            transaction
                .commit()
                .await
                .map_err(ResolveIdentityError::provider)?;
            return Ok(ResolvedIdentity {
                user_id: UserId::from_uuid(prior.user_id),
                display_name: prior.display_name,
                idempotency_id,
            });
        }
        sqlx::query("SELECT set_config('hephaestus.occurrence_id', $1, true)")
            .bind(idempotency_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(ResolveIdentityError::provider)?;
        let identity = map_verified_in_transaction(
            &mut transaction,
            &request.verified,
            request.request_id,
            None,
        )
        .await
        .map_err(map_resolution_error)?;
        let display_name = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
            .bind(identity.user_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(ResolveIdentityError::provider)?;
        transaction
            .commit()
            .await
            .map_err(ResolveIdentityError::provider)?;
        Ok(ResolvedIdentity {
            user_id: identity.user_id,
            display_name,
            idempotency_id,
        })
    }
}

#[async_trait]
impl IdentityBootstrapper for PostgresIdentityStore {
    async fn bootstrap_identity(
        &self,
        identity: BootstrapIdentity,
    ) -> Result<(), BootstrapIdentityError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(BootstrapIdentityError::provider)?;
        sqlx::query(
            "INSERT INTO users (id, display_name)
             VALUES ($1, $2)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(identity.user_id.as_uuid())
        .bind(identity.display_name)
        .execute(&mut *transaction)
        .await
        .map_err(BootstrapIdentityError::provider)?;
        let mapped_user_id: Uuid = sqlx::query_scalar(
            "INSERT INTO external_identities
             (user_id, issuer, subject, provider_metadata)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (issuer, subject) DO UPDATE SET issuer = EXCLUDED.issuer
             RETURNING user_id",
        )
        .bind(identity.user_id.as_uuid())
        .bind(identity.issuer)
        .bind(identity.subject)
        .bind(identity.provider_metadata)
        .fetch_one(&mut *transaction)
        .await
        .map_err(BootstrapIdentityError::provider)?;
        if mapped_user_id != identity.user_id.as_uuid() {
            return Err(BootstrapIdentityError::Conflict);
        }
        transaction
            .commit()
            .await
            .map_err(BootstrapIdentityError::provider)
    }
}

async fn map_verified_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    verified: &VerifiedExternalIdentity,
    request_id: RequestId,
    trace_id: Option<&str>,
) -> Result<AuthenticatedIdentity, IdentityMappingError> {
    let row = sqlx::query_as::<_, IdentityMappingRow>(
        "SELECT users.id AS user_id, users.status
         FROM external_identities
         JOIN users ON users.id = external_identities.user_id
         WHERE external_identities.issuer = $1
           AND external_identities.subject = $2",
    )
    .bind(&verified.issuer)
    .bind(&verified.subject)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(IdentityMappingError::provider)?
    .ok_or(IdentityMappingError::Unmapped)?;
    if row.status != "active" {
        return Err(IdentityMappingError::Inactive);
    }
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true)",
    )
    .bind(row.user_id.to_string())
    .bind(request_id.to_string())
    .execute(&mut **transaction)
    .await
    .map_err(IdentityMappingError::provider)?;
    sqlx::query(
        "INSERT INTO user_profiles (user_id, validated_claims)
         VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE
         SET validated_claims = EXCLUDED.validated_claims, updated_at = now()",
    )
    .bind(row.user_id)
    .bind(&verified.claims)
    .execute(&mut **transaction)
    .await
    .map_err(IdentityMappingError::provider)?;
    let mut identity = AuthenticatedIdentity::new(
        UserId::from_uuid(row.user_id),
        verified.issuer.clone(),
        verified.subject.clone(),
        verified.claims.clone(),
        request_id,
    );
    identity.trace_id = trace_id.map(str::to_owned);
    Ok(identity)
}

fn map_resolution_error(error: IdentityMappingError) -> ResolveIdentityError {
    match error {
        IdentityMappingError::Unmapped | IdentityMappingError::Inactive => {
            ResolveIdentityError::PermissionDenied
        }
        IdentityMappingError::Provider(error) => ResolveIdentityError::Provider(error),
    }
}

#[derive(FromRow)]
struct IdentityMappingRow {
    user_id: Uuid,
    status: String,
}

#[derive(FromRow)]
struct PriorResolutionRow {
    user_id: Uuid,
    display_name: String,
    validated_claims: Value,
}

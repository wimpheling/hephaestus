//! PostgreSQL-native authorization and request actor context.

use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, Authorizer, AuthzError, ObjectRef, Permission, Subject};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::RunId;
use sqlx::{PgPool, Postgres, Transaction};

/// Canonical authorization model revision recorded with audit events.
pub const AUTHORIZATION_MODEL_VERSION: &str =
    "melange-0.8.5:76e7043ed8a534103adff658f24be57485646163ab73a8e33c2dc6d56c91d298";

/// Specialized Mélange authorization provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct PostgresMelangeAuthorizer;

#[async_trait]
impl Authorizer for PostgresMelangeAuthorizer {
    async fn check(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        subject: Subject,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<AuthorizationDecision, AuthzError> {
        let context: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT current_setting('hephaestus.subject_type', true),
                    current_setting('hephaestus.actor_id', true)",
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(AuthzError::Database)?;
        let subject_id = subject.id();
        if context.0.as_deref().unwrap_or("user") != subject.object_type()
            || context.1.as_deref() != Some(subject_id.as_str())
        {
            return Err(AuthzError::MissingActorContext);
        }
        let allowed: bool = sqlx::query_scalar("SELECT check_permission($1, $2, $3, $4, $5) = 1")
            .bind(subject.object_type())
            .bind(subject_id)
            .bind(permission.as_str())
            .bind(object.object_type.as_str())
            .bind(object.id.to_string())
            .fetch_one(&mut **tx)
            .await
            .map_err(AuthzError::Database)?;
        Ok(if allowed {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        })
    }
}

/// Begins a request transaction and sets transaction-local actor provenance.
///
/// # Errors
///
/// Returns a database error when the transaction or context cannot be created.
pub async fn begin_actor_transaction<'pool>(
    pool: &'pool PgPool,
    identity: &AuthenticatedIdentity,
) -> Result<Transaction<'pool, Postgres>, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true)",
    )
    .bind(identity.user_id.to_string())
    .bind(identity.request_id.to_string())
    .execute(&mut *transaction)
    .await?;
    Ok(transaction)
}

/// Begins a transaction scoped to an already-authenticated exact runtime.
///
/// The broker must authenticate the opaque runtime credential and match its
/// stored hash before calling this helper.
///
/// # Errors
///
/// Returns a database error when the transaction context cannot be created.
pub async fn begin_runtime_transaction(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'run', true)",
    )
    .bind(run_id.to_string())
    .execute(&mut *transaction)
    .await?;
    Ok(transaction)
}

/// Writes one command-level authorization audit event.
///
/// # Errors
///
/// Returns a database error when the audit record cannot be persisted.
pub async fn audit_decision(
    transaction: &mut Transaction<'_, Postgres>,
    actor_id: UserId,
    permission: Permission,
    object: ObjectRef,
    decision: AuthorizationDecision,
    request_id: RequestId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO authorization_audit_events
         (id, actor_id, permission, object_type, object_id, decision,
          request_id, authorization_model_version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(actor_id.as_uuid())
    .bind(permission.as_str())
    .bind(object.object_type.as_str())
    .bind(object.id)
    .bind(if decision.is_allowed() {
        "allow"
    } else {
        "deny"
    })
    .bind(request_id.as_uuid())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

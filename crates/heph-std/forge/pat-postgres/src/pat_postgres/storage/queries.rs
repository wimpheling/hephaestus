//! `SQLx` operations for PAT lifecycle persistence and audit rows.

use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use pat_domain::{PersonalAccessTokenId, PersonalAccessTokenRecord};
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use super::super::model::PersonalAccessTokenServiceError;
use super::rows::PersonalAccessTokenRow;

pub(in crate::pat_postgres) async fn append_identity_profile_event(
    transaction: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    token_id: PersonalAccessTokenId,
    change_kind: &str,
    safe_state: &str,
) -> Result<(), PersonalAccessTokenServiceError> {
    sqlx::query(
        "SELECT append_application_event(
             $1, 'identity', $2, 'identity_profile', $2,
             'identity.profile_changed', $3, $4, $5, NULL
         )",
    )
    .bind(identity.request_id.as_uuid())
    .bind(identity.user_id.as_uuid())
    .bind(change_kind)
    .bind(safe_state)
    .bind(token_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}

pub(in crate::pat_postgres) async fn begin_actor_transaction<'a>(
    pool: &'a PgPool,
    identity: &AuthenticatedIdentity,
) -> Result<Transaction<'a, Postgres>, PersonalAccessTokenServiceError> {
    let mut transaction = pool.begin().await.map_err(storage)?;
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true)",
    )
    .bind(identity.user_id.to_string())
    .bind(identity.request_id.to_string())
    .execute(&mut *transaction)
    .await
    .map_err(storage)?;
    Ok(transaction)
}

pub(in crate::pat_postgres) async fn insert_record(
    transaction: &mut Transaction<'_, Postgres>,
    record: &PersonalAccessTokenRecord,
    rotated_from_id: Option<PersonalAccessTokenId>,
) -> Result<(), PersonalAccessTokenServiceError> {
    let operations = record
        .scope()
        .operations()
        .iter()
        .copied()
        .map(operation_name)
        .collect::<Vec<_>>();
    let restrictions = record
        .scope()
        .repository_restrictions()
        .map(|repositories| {
            repositories
                .iter()
                .copied()
                .map(RepositoryId::as_uuid)
                .collect::<Vec<_>>()
        });
    let verifier = record.verifier();
    sqlx::query(
        "INSERT INTO developer_personal_access_tokens
            (id, verifier_version, verifier_digest, owner_user_id, label,
             git_operations, repository_restrictions, created_at, expires_at,
             creation_request_id, rotated_from_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(record.id().as_uuid())
    .bind(i16::try_from(verifier.version()).map_err(storage)?)
    .bind(verifier.digest().as_slice())
    .bind(record.owner_user_id().as_uuid())
    .bind(record.label().as_str())
    .bind(&operations)
    .bind(restrictions)
    .bind(record.created_at())
    .bind(record.expires_at())
    .bind(record.creation_request_id().as_uuid())
    .bind(rotated_from_id.map(PersonalAccessTokenId::as_uuid))
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}

pub(in crate::pat_postgres) async fn find_owned_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    owner_user_id: UserId,
    token_id: PersonalAccessTokenId,
) -> Result<Option<PersonalAccessTokenRecord>, PersonalAccessTokenServiceError> {
    find_row_for_update(transaction, token_id, Some(owner_user_id))
        .await?
        .map(PersonalAccessTokenRow::into_record)
        .transpose()
}

pub(in crate::pat_postgres) async fn find_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    token_id: PersonalAccessTokenId,
) -> Result<Option<PersonalAccessTokenRow>, PersonalAccessTokenServiceError> {
    sqlx::query_as::<_, PersonalAccessTokenRow>(
        "SELECT token.id, token.verifier_version, token.verifier_digest,
                token.owner_user_id, token.label, token.git_operations,
                token.repository_restrictions, token.created_at,
                token.expires_at, token.revoked_at, token.last_used_at,
                token.creation_request_id
         FROM developer_personal_access_tokens token
         JOIN users ON users.id = token.owner_user_id
         WHERE token.id = $1 AND users.status = 'active'
         FOR UPDATE OF token",
    )
    .bind(token_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)
}

pub(in crate::pat_postgres) async fn find_row_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    token_id: PersonalAccessTokenId,
    owner_user_id: Option<UserId>,
) -> Result<Option<PersonalAccessTokenRow>, PersonalAccessTokenServiceError> {
    sqlx::query_as::<_, PersonalAccessTokenRow>(
        "SELECT id, verifier_version, verifier_digest, owner_user_id, label,
                git_operations, repository_restrictions, created_at,
                expires_at, revoked_at, last_used_at, creation_request_id
         FROM developer_personal_access_tokens
         WHERE id = $1 AND ($2::uuid IS NULL OR owner_user_id = $2)
         FOR UPDATE",
    )
    .bind(token_id.as_uuid())
    .bind(owner_user_id.map(UserId::as_uuid))
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)
}

pub(in crate::pat_postgres) async fn set_revoked(
    transaction: &mut Transaction<'_, Postgres>,
    token_id: PersonalAccessTokenId,
    owner_user_id: UserId,
    revoked_at: OffsetDateTime,
    request_id: RequestId,
) -> Result<(), PersonalAccessTokenServiceError> {
    let result = sqlx::query(
        "UPDATE developer_personal_access_tokens
         SET revoked_at = $3, revocation_request_id = $4
         WHERE id = $1 AND owner_user_id = $2 AND revoked_at IS NULL",
    )
    .bind(token_id.as_uuid())
    .bind(owner_user_id.as_uuid())
    .bind(revoked_at)
    .bind(request_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    if result.rows_affected() != 1 {
        return Err(PersonalAccessTokenServiceError::InvalidLifecycle);
    }
    Ok(())
}

pub(in crate::pat_postgres) async fn update_last_used(
    transaction: &mut Transaction<'_, Postgres>,
    token_id: PersonalAccessTokenId,
    used_at: OffsetDateTime,
) -> Result<(), PersonalAccessTokenServiceError> {
    let result = sqlx::query(
        "UPDATE developer_personal_access_tokens
         SET last_used_at = $2
         WHERE id = $1 AND revoked_at IS NULL AND expires_at > $2
           AND (last_used_at IS NULL OR last_used_at <= $2)",
    )
    .bind(token_id.as_uuid())
    .bind(used_at)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    if result.rows_affected() != 1 {
        return Err(PersonalAccessTokenServiceError::InvalidCredential);
    }
    Ok(())
}

// Audit arguments mirror the append-only schema so omission of an exact
// operation, repository, or rotation peer remains explicit at call sites.
#[allow(clippy::too_many_arguments)]
pub(in crate::pat_postgres) async fn insert_audit(
    transaction: &mut Transaction<'_, Postgres>,
    token_id: PersonalAccessTokenId,
    owner_user_id: UserId,
    event_type: &str,
    request_id: RequestId,
    repository_id: Option<RepositoryId>,
    operation: Option<GitOperation>,
    related_token_id: Option<PersonalAccessTokenId>,
    occurred_at: OffsetDateTime,
) -> Result<(), PersonalAccessTokenServiceError> {
    sqlx::query(
        "INSERT INTO personal_access_token_audit_events
            (id, token_id, owner_user_id, event_type, request_id,
             repository_id, git_operation, related_token_id, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(Uuid::new_v4())
    .bind(token_id.as_uuid())
    .bind(owner_user_id.as_uuid())
    .bind(event_type)
    .bind(request_id.as_uuid())
    .bind(repository_id.map(RepositoryId::as_uuid))
    .bind(operation.map(operation_name))
    .bind(related_token_id.map(PersonalAccessTokenId::as_uuid))
    .bind(occurred_at)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}

const fn operation_name(operation: GitOperation) -> &'static str {
    match operation {
        GitOperation::Discover => "discover",
        GitOperation::Fetch => "fetch",
        GitOperation::Receive => "receive",
    }
}

pub(in crate::pat_postgres) fn storage(
    error: impl std::fmt::Display,
) -> PersonalAccessTokenServiceError {
    let _ = error;
    PersonalAccessTokenServiceError::Persistence
}

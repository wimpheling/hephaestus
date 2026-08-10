//! PostgreSQL-backed developer personal access token issuance and lifecycle.
//!
//! The service generates bearer material in memory, persists only its
//! domain-separated verifier, and exposes verifier-free metadata to callers.

use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use pat_domain::{
    PersonalAccessToken, PersonalAccessTokenAuthorizationError, PersonalAccessTokenError,
    PersonalAccessTokenId, PersonalAccessTokenLabel, PersonalAccessTokenMetadata,
    PersonalAccessTokenRecord, PersonalAccessTokenScope, PersonalAccessTokenVerifier,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

/// Parameters for issuing a developer personal access token.
#[derive(Debug, Clone)]
pub struct CreatePersonalAccessToken {
    /// Safe user-visible label.
    pub label: PersonalAccessTokenLabel,
    /// Exact Git operation and optional repository narrowing.
    pub scope: PersonalAccessTokenScope,
    /// Exclusive token expiry, subject to the domain lifetime ceiling.
    pub expires_at: OffsetDateTime,
}

/// Parameters for atomically revoking and replacing a PAT.
#[derive(Debug, Clone)]
pub struct RotatePersonalAccessToken {
    /// Existing PAT owned by the authenticated user.
    pub token_id: PersonalAccessTokenId,
    /// Safe label for the replacement.
    pub label: PersonalAccessTokenLabel,
    /// Exact scope for the replacement; it is not inherited implicitly.
    pub scope: PersonalAccessTokenScope,
    /// Exclusive replacement expiry.
    pub expires_at: OffsetDateTime,
}

/// One newly issued plaintext value and its verifier-free metadata.
///
/// The token is intentionally neither cloneable nor serializable as
/// plaintext. Callers may invoke [`PersonalAccessToken::expose`] only at the
/// one-time delivery boundary.
pub struct IssuedPersonalAccessToken {
    /// Ephemeral plaintext token.
    pub token: PersonalAccessToken,
    /// Safe metadata suitable for a response or listing.
    pub metadata: PersonalAccessTokenMetadata,
}

impl std::fmt::Debug for IssuedPersonalAccessToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedPersonalAccessToken")
            .field("token", &self.token)
            .field("metadata", &self.metadata)
            .finish()
    }
}

/// Successful token-local Git authentication result.
///
/// Git HTTP must still check the owner's current live authorization for the
/// repository after receiving this result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatedPersonalAccessToken {
    /// PAT owner whose live repository authority must be evaluated.
    pub owner_user_id: UserId,
    /// Stable token identifier for audit correlation.
    pub token_id: PersonalAccessTokenId,
}

/// Safe service error that never includes plaintext bearer material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PersonalAccessTokenServiceError {
    /// Requested owner-visible token does not exist.
    #[error("personal access token not found")]
    NotFound,
    /// Credential lookup, verifier, lifecycle, or exact scope did not pass.
    #[error("invalid personal access token credential")]
    InvalidCredential,
    /// The requested lifecycle transition is invalid.
    #[error("invalid personal access token lifecycle transition")]
    InvalidLifecycle,
    /// Issuance input violates the PAT domain contract.
    #[error("invalid personal access token request")]
    InvalidRequest,
    /// Cryptographically secure bearer generation failed.
    #[error("personal access token generation failed")]
    Entropy,
    /// Durable storage did not satisfy the expected contract.
    #[error("personal access token persistence failed")]
    Persistence,
}

/// `PostgreSQL` PAT service usable from authenticated application and trusted
/// Git authentication boundaries.
#[derive(Clone)]
pub struct PostgresPersonalAccessTokenService {
    pool: PgPool,
}

impl PostgresPersonalAccessTokenService {
    /// Creates a service over an appropriately role-scoped pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Issues a PAT for the authenticated user and returns plaintext once.
    ///
    /// # Errors
    ///
    /// Returns a safe request, entropy, or persistence error.
    pub async fn create(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreatePersonalAccessToken,
    ) -> Result<IssuedPersonalAccessToken, PersonalAccessTokenServiceError> {
        let issued_at = postgres_timestamp(OffsetDateTime::now_utc())?;
        let expires_at = postgres_timestamp(command.expires_at)?;
        let token = generate_token()?;
        let record = PersonalAccessTokenRecord::issue(
            &token,
            identity.user_id,
            command.label,
            command.scope,
            issued_at,
            expires_at,
            identity.request_id,
        )
        .map_err(request_error)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity).await?;
        insert_record(&mut transaction, &record, None).await?;
        insert_audit(
            &mut transaction,
            record.id(),
            record.owner_user_id(),
            "issued",
            identity.request_id,
            None,
            None,
            None,
            issued_at,
        )
        .await?;
        append_identity_profile_event(&mut transaction, identity, record.id(), "created", "active")
            .await?;
        transaction.commit().await.map_err(storage)?;
        Ok(IssuedPersonalAccessToken {
            metadata: record.metadata(),
            token,
        })
    }

    /// Lists only verifier-free metadata owned by the authenticated user.
    ///
    /// # Errors
    ///
    /// Returns a persistence error if stored state is invalid or unavailable.
    pub async fn list(
        &self,
        identity: &AuthenticatedIdentity,
    ) -> Result<Vec<PersonalAccessTokenMetadata>, PersonalAccessTokenServiceError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity).await?;
        let rows = sqlx::query_as::<_, PersonalAccessTokenMetadataRow>(
            "SELECT id, owner_user_id, label, git_operations,
                    repository_restrictions, created_at, expires_at,
                    revoked_at, last_used_at, creation_request_id
             FROM developer_personal_access_tokens
             WHERE owner_user_id = $1
             ORDER BY created_at DESC, id",
        )
        .bind(identity.user_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        rows.into_iter()
            .map(PersonalAccessTokenMetadataRow::into_metadata)
            .collect()
    }

    /// Irreversibly revokes an owned PAT with immediate effect.
    ///
    /// # Errors
    ///
    /// Returns `NotFound`, `InvalidLifecycle`, or a persistence error.
    pub async fn revoke(
        &self,
        identity: &AuthenticatedIdentity,
        token_id: PersonalAccessTokenId,
    ) -> Result<PersonalAccessTokenMetadata, PersonalAccessTokenServiceError> {
        let revoked_at = postgres_timestamp(OffsetDateTime::now_utc())?;
        let mut transaction = begin_actor_transaction(&self.pool, identity).await?;
        let mut record = find_owned_for_update(&mut transaction, identity.user_id, token_id)
            .await?
            .ok_or(PersonalAccessTokenServiceError::NotFound)?;
        record.revoke(revoked_at).map_err(lifecycle_error)?;
        set_revoked(
            &mut transaction,
            token_id,
            identity.user_id,
            revoked_at,
            identity.request_id,
        )
        .await?;
        insert_audit(
            &mut transaction,
            token_id,
            identity.user_id,
            "revoked",
            identity.request_id,
            None,
            None,
            None,
            revoked_at,
        )
        .await?;
        append_identity_profile_event(
            &mut transaction,
            identity,
            token_id,
            "state_changed",
            "revoked",
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        Ok(record.metadata())
    }

    /// Atomically revokes an owned PAT and issues a separately scoped PAT.
    ///
    /// # Errors
    ///
    /// Returns a safe request, lifecycle, entropy, or persistence error.
    pub async fn rotate(
        &self,
        identity: &AuthenticatedIdentity,
        command: RotatePersonalAccessToken,
    ) -> Result<IssuedPersonalAccessToken, PersonalAccessTokenServiceError> {
        let rotated_at = postgres_timestamp(OffsetDateTime::now_utc())?;
        let expires_at = postgres_timestamp(command.expires_at)?;
        let token = generate_token()?;
        let replacement = PersonalAccessTokenRecord::issue(
            &token,
            identity.user_id,
            command.label,
            command.scope,
            rotated_at,
            expires_at,
            identity.request_id,
        )
        .map_err(request_error)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity).await?;
        let mut previous =
            find_owned_for_update(&mut transaction, identity.user_id, command.token_id)
                .await?
                .ok_or(PersonalAccessTokenServiceError::NotFound)?;
        previous.revoke(rotated_at).map_err(lifecycle_error)?;
        set_revoked(
            &mut transaction,
            command.token_id,
            identity.user_id,
            rotated_at,
            identity.request_id,
        )
        .await?;
        insert_record(&mut transaction, &replacement, Some(command.token_id)).await?;
        insert_audit(
            &mut transaction,
            command.token_id,
            identity.user_id,
            "rotated",
            identity.request_id,
            None,
            None,
            Some(replacement.id()),
            rotated_at,
        )
        .await?;
        insert_audit(
            &mut transaction,
            replacement.id(),
            identity.user_id,
            "rotated",
            identity.request_id,
            None,
            None,
            Some(command.token_id),
            rotated_at,
        )
        .await?;
        append_identity_profile_event(
            &mut transaction,
            identity,
            replacement.id(),
            "updated",
            "active",
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        Ok(IssuedPersonalAccessToken {
            metadata: replacement.metadata(),
            token,
        })
    }

    /// Authenticates exact token, operation, and repository scope and records
    /// successful use atomically.
    ///
    /// This performs token-local authentication only. The Git boundary must
    /// subsequently evaluate the owner's current repository authorization.
    ///
    /// # Errors
    ///
    /// Returns only a generic credential denial for absent, malformed,
    /// expired, revoked, or out-of-scope credentials.
    pub async fn authenticate(
        &self,
        token: &PersonalAccessToken,
        operation: GitOperation,
        repository_id: RepositoryId,
        request_id: RequestId,
    ) -> Result<AuthenticatedPersonalAccessToken, PersonalAccessTokenServiceError> {
        let authenticated_at = postgres_timestamp(OffsetDateTime::now_utc())?;
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let row = find_for_update(&mut transaction, token.id())
            .await?
            .ok_or(PersonalAccessTokenServiceError::InvalidCredential)?;
        let mut record = row.into_record()?;
        record
            .authorize_at(
                token,
                record.owner_user_id(),
                operation,
                repository_id,
                authenticated_at,
            )
            .map_err(authentication_error)?;
        record
            .record_use(authenticated_at)
            .map_err(authentication_lifecycle_error)?;
        update_last_used(&mut transaction, token.id(), authenticated_at).await?;
        insert_audit(
            &mut transaction,
            token.id(),
            record.owner_user_id(),
            "authenticated",
            request_id,
            Some(repository_id),
            Some(operation),
            None,
            authenticated_at,
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        Ok(AuthenticatedPersonalAccessToken {
            owner_user_id: record.owner_user_id(),
            token_id: record.id(),
        })
    }
}

async fn append_identity_profile_event(
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

#[derive(FromRow)]
struct PersonalAccessTokenRow {
    id: Uuid,
    verifier_version: i16,
    verifier_digest: Vec<u8>,
    owner_user_id: Uuid,
    label: String,
    git_operations: Vec<String>,
    repository_restrictions: Option<Vec<Uuid>>,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    last_used_at: Option<OffsetDateTime>,
    creation_request_id: Uuid,
}

#[derive(FromRow)]
struct PersonalAccessTokenMetadataRow {
    id: Uuid,
    owner_user_id: Uuid,
    label: String,
    git_operations: Vec<String>,
    repository_restrictions: Option<Vec<Uuid>>,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    last_used_at: Option<OffsetDateTime>,
    creation_request_id: Uuid,
}

impl PersonalAccessTokenRow {
    fn into_record(self) -> Result<PersonalAccessTokenRecord, PersonalAccessTokenServiceError> {
        let version = u16::try_from(self.verifier_version).map_err(storage)?;
        let digest = self
            .verifier_digest
            .try_into()
            .map_err(|_| PersonalAccessTokenServiceError::Persistence)?;
        let scope = parse_scope(self.git_operations, self.repository_restrictions)?;
        PersonalAccessTokenRecord::restore(
            PersonalAccessTokenId::from_uuid(self.id),
            PersonalAccessTokenVerifier::from_digest(version, digest),
            UserId::from_uuid(self.owner_user_id),
            PersonalAccessTokenLabel::parse(self.label)
                .map_err(|_| PersonalAccessTokenServiceError::Persistence)?,
            scope,
            self.created_at,
            self.expires_at,
            self.revoked_at,
            self.last_used_at,
            RequestId::from_uuid(self.creation_request_id),
        )
        .map_err(|_| PersonalAccessTokenServiceError::Persistence)
    }
}

impl PersonalAccessTokenMetadataRow {
    fn into_metadata(self) -> Result<PersonalAccessTokenMetadata, PersonalAccessTokenServiceError> {
        Ok(PersonalAccessTokenMetadata {
            id: PersonalAccessTokenId::from_uuid(self.id),
            owner_user_id: UserId::from_uuid(self.owner_user_id),
            label: PersonalAccessTokenLabel::parse(self.label)
                .map_err(|_| PersonalAccessTokenServiceError::Persistence)?,
            scope: parse_scope(self.git_operations, self.repository_restrictions)?,
            created_at: self.created_at,
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
            last_used_at: self.last_used_at,
            creation_request_id: RequestId::from_uuid(self.creation_request_id),
        })
    }
}

fn parse_scope(
    operations: Vec<String>,
    repository_restrictions: Option<Vec<Uuid>>,
) -> Result<PersonalAccessTokenScope, PersonalAccessTokenServiceError> {
    let operations = operations
        .into_iter()
        .map(|operation| parse_operation(&operation))
        .collect::<Result<Vec<_>, _>>()?;
    let restrictions = repository_restrictions.map(|repositories| {
        repositories
            .into_iter()
            .map(RepositoryId::from_uuid)
            .collect::<Vec<_>>()
    });
    PersonalAccessTokenScope::new(operations, restrictions)
        .map_err(|_| PersonalAccessTokenServiceError::Persistence)
}

fn generate_token() -> Result<PersonalAccessToken, PersonalAccessTokenServiceError> {
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret).map_err(|_| PersonalAccessTokenServiceError::Entropy)?;
    Ok(PersonalAccessToken::from_secret(
        PersonalAccessTokenId::new(),
        secret,
    ))
}

fn postgres_timestamp(
    value: OffsetDateTime,
) -> Result<OffsetDateTime, PersonalAccessTokenServiceError> {
    let microsecond_precision = value.nanosecond() / 1_000 * 1_000;
    value
        .replace_nanosecond(microsecond_precision)
        .map_err(|_| PersonalAccessTokenServiceError::InvalidRequest)
}

async fn begin_actor_transaction<'a>(
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

async fn insert_record(
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

async fn find_owned_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    owner_user_id: UserId,
    token_id: PersonalAccessTokenId,
) -> Result<Option<PersonalAccessTokenRecord>, PersonalAccessTokenServiceError> {
    find_row_for_update(transaction, token_id, Some(owner_user_id))
        .await?
        .map(PersonalAccessTokenRow::into_record)
        .transpose()
}

async fn find_for_update(
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

async fn find_row_for_update(
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

async fn set_revoked(
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

async fn update_last_used(
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
async fn insert_audit(
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

fn parse_operation(operation: &str) -> Result<GitOperation, PersonalAccessTokenServiceError> {
    match operation {
        "discover" => Ok(GitOperation::Discover),
        "fetch" => Ok(GitOperation::Fetch),
        "receive" => Ok(GitOperation::Receive),
        _ => Err(PersonalAccessTokenServiceError::Persistence),
    }
}

const fn request_error(_: PersonalAccessTokenError) -> PersonalAccessTokenServiceError {
    PersonalAccessTokenServiceError::InvalidRequest
}

const fn lifecycle_error(_: PersonalAccessTokenError) -> PersonalAccessTokenServiceError {
    PersonalAccessTokenServiceError::InvalidLifecycle
}

const fn authentication_error(
    _: PersonalAccessTokenAuthorizationError,
) -> PersonalAccessTokenServiceError {
    PersonalAccessTokenServiceError::InvalidCredential
}

const fn authentication_lifecycle_error(
    _: PersonalAccessTokenError,
) -> PersonalAccessTokenServiceError {
    PersonalAccessTokenServiceError::InvalidCredential
}

fn storage(error: impl std::fmt::Display) -> PersonalAccessTokenServiceError {
    let _ = error;
    PersonalAccessTokenServiceError::Persistence
}

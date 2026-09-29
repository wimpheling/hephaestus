//! PAT lifecycle operations and token-local authentication.

use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use identity_domain::{AuthenticatedIdentity, RequestId};
use pat_domain::{
    PersonalAccessToken, PersonalAccessTokenAuthorizationError, PersonalAccessTokenError,
    PersonalAccessTokenId, PersonalAccessTokenMetadata, PersonalAccessTokenRecord,
};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::model::{
    AuthenticatedPersonalAccessToken, CreatePersonalAccessToken, IssuedPersonalAccessToken,
    PersonalAccessTokenServiceError, PostgresPersonalAccessTokenService, RotatePersonalAccessToken,
};
use super::storage::{
    PersonalAccessTokenMetadataRow, append_identity_profile_event, begin_actor_transaction,
    find_for_update, find_owned_for_update, insert_audit, insert_record, set_revoked, storage,
    update_last_used,
};

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

use super::PostgresBrowserSessionStore;
use identity_application::{
    CreateBrowserSession, CreateBrowserSessionError, CreatedBrowserSession,
};
use identity_domain::{
    AuthenticatedIdentity, BrowserSessionId, BrowserSessionMetadata,
    DEFAULT_BROWSER_SESSION_TTL_SECONDS, UserId, actor_idempotency_id,
    browser_session_identity_binding_digest, browser_session_sid_digest,
};
use sqlx::types::time::OffsetDateTime;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

impl PostgresBrowserSessionStore {
    /// Creates or replays one session from an already verified identity.
    ///
    /// The mapping and user rows are locked before deriving idempotency and
    /// capturing database timestamps. A successful insertion and its safe
    /// identity event commit atomically.
    ///
    /// # Errors
    ///
    /// Returns a typed denial, replay conflict, inactive replay, or opaque
    /// persistence failure.
    pub async fn create_browser_session(
        &self,
        command: CreateBrowserSession,
    ) -> Result<CreatedBrowserSession, CreateBrowserSessionError> {
        let mut transaction = self
            .worker_pool
            .begin()
            .await
            .map_err(|_| CreateBrowserSessionError::Unavailable)?;
        let mapping = sqlx::query_as::<_, IdentityMappingRow>(
            "SELECT external_identity.user_id, user_row.status
             FROM external_identities AS external_identity
             JOIN users AS user_row ON user_row.id = external_identity.user_id
             WHERE external_identity.issuer = $1
               AND external_identity.subject = $2
             FOR UPDATE OF external_identity, user_row",
        )
        .bind(&command.verified.issuer)
        .bind(&command.verified.subject)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| CreateBrowserSessionError::Unavailable)?
        .ok_or(CreateBrowserSessionError::PermissionDenied)?;
        if mapping.status != "active" {
            return Err(CreateBrowserSessionError::PermissionDenied);
        }

        let user_id = UserId::from_uuid(mapping.user_id);
        let idempotency_id =
            actor_idempotency_id(mapping.user_id.as_bytes(), &command.idempotency_seed);
        let verified = AuthenticatedIdentity::new(
            user_id,
            command.verified.issuer.clone(),
            command.verified.subject.clone(),
            serde_json::Value::Null,
            command.request_id,
        );
        let identity_binding_digest = browser_session_identity_binding_digest(&verified)
            .as_bytes()
            .to_vec();
        let sid_digest = browser_session_sid_digest(command.sid).as_bytes().to_vec();

        set_event_context(
            &mut transaction,
            user_id,
            command.request_id,
            idempotency_id,
        )
        .await?;
        let inserted = sqlx::query_as::<_, InsertedSessionRow>(
            "INSERT INTO human_browser_sessions (
                 id, sid_digest, creation_idempotency_id, creation_request_id,
                 identity_binding_digest, user_id, issued_at, expires_at
             ) VALUES (
                 gen_random_uuid(), $1, $2, $3, $4, $5,
                 statement_timestamp(),
                 statement_timestamp() + ($6::bigint * interval '1 second')
             )
             ON CONFLICT DO NOTHING
             RETURNING id, user_id, issued_at, expires_at, revoked_at",
        )
        .bind(&sid_digest)
        .bind(idempotency_id.as_uuid())
        .bind(command.request_id.as_uuid())
        .bind(&identity_binding_digest)
        .bind(mapping.user_id)
        .bind(DEFAULT_BROWSER_SESSION_TTL_SECONDS)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| CreateBrowserSessionError::Unavailable)?;

        let Some(inserted) = inserted else {
            let replayed = replay_existing(
                &mut transaction,
                idempotency_id,
                mapping.user_id,
                &sid_digest,
                &identity_binding_digest,
            )
            .await;
            let replayed = replayed?;
            transaction
                .commit()
                .await
                .map_err(|_| CreateBrowserSessionError::Unavailable)?;
            return Ok(replayed);
        };
        let metadata = session_metadata(&inserted)?;
        append_identity_event(&mut transaction, mapping.user_id, idempotency_id).await?;
        transaction
            .commit()
            .await
            .map_err(|_| CreateBrowserSessionError::Unavailable)?;
        Ok(CreatedBrowserSession {
            metadata,
            idempotency_id,
        })
    }
}
async fn replay_existing(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_id: identity_domain::RequestId,
    user_id: Uuid,
    sid_digest: &[u8],
    identity_binding_digest: &[u8],
) -> Result<CreatedBrowserSession, CreateBrowserSessionError> {
    let rows = sqlx::query_as::<_, ExistingSessionRow>(
        "SELECT id, user_id, sid_digest, creation_idempotency_id,
                identity_binding_digest, issued_at, expires_at, revoked_at,
                revoked_at IS NULL
                    AND issued_at <= statement_timestamp()
                    AND expires_at > statement_timestamp() AS active
         FROM human_browser_sessions
         WHERE creation_idempotency_id = $1
         FOR UPDATE",
    )
    .bind(idempotency_id.as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| CreateBrowserSessionError::Unavailable)?;
    if rows.len() != 1 {
        return Err(CreateBrowserSessionError::IdempotencyConflict);
    }
    let row = rows.into_iter().next().expect("one row checked");
    if row.user_id != user_id
        || row.creation_idempotency_id != idempotency_id.as_uuid()
        || row.sid_digest != sid_digest
        || row.identity_binding_digest != identity_binding_digest
    {
        return Err(CreateBrowserSessionError::IdempotencyConflict);
    }
    if !row.active {
        return Err(CreateBrowserSessionError::InactiveReplay);
    }
    let metadata = BrowserSessionMetadata::new(
        BrowserSessionId::from_uuid(row.id),
        UserId::from_uuid(row.user_id),
        row.issued_at,
        row.expires_at,
        row.revoked_at,
    )
    .ok_or(CreateBrowserSessionError::Unavailable)?;
    Ok(CreatedBrowserSession {
        metadata,
        idempotency_id,
    })
}

async fn set_event_context(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: UserId,
    request_id: identity_domain::RequestId,
    idempotency_id: identity_domain::RequestId,
) -> Result<(), CreateBrowserSessionError> {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true),
                set_config('hephaestus.occurrence_id', $3, true)",
    )
    .bind(user_id.to_string())
    .bind(request_id.to_string())
    .bind(idempotency_id.to_string())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|_| CreateBrowserSessionError::Unavailable)
}

async fn append_identity_event(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    idempotency_id: identity_domain::RequestId,
) -> Result<(), CreateBrowserSessionError> {
    sqlx::query(
        "SELECT event_id, cursor, aggregate_version
         FROM append_application_event(
             $1, 'identity', $2, 'identity_profile', $2,
             'identity.profile_changed', 'updated', 'active', NULL, NULL
         )",
    )
    .bind(idempotency_id.as_uuid())
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|_| CreateBrowserSessionError::Unavailable)
}

fn session_metadata(
    row: &InsertedSessionRow,
) -> Result<BrowserSessionMetadata, CreateBrowserSessionError> {
    BrowserSessionMetadata::new(
        BrowserSessionId::from_uuid(row.id),
        UserId::from_uuid(row.user_id),
        row.issued_at,
        row.expires_at,
        row.revoked_at,
    )
    .ok_or(CreateBrowserSessionError::Unavailable)
}

#[derive(FromRow)]
struct IdentityMappingRow {
    user_id: Uuid,
    status: String,
}

#[derive(FromRow)]
struct InsertedSessionRow {
    id: Uuid,
    user_id: Uuid,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
struct ExistingSessionRow {
    id: Uuid,
    user_id: Uuid,
    sid_digest: Vec<u8>,
    creation_idempotency_id: Uuid,
    identity_binding_digest: Vec<u8>,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    active: bool,
}

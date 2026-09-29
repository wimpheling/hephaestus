use super::PostgresBrowserSessionStore;
use identity_application::{
    RevokeBrowserSession, RevokeBrowserSessionError, RevokedBrowserSession,
};
use identity_domain::{UserId, actor_idempotency_id, browser_session_sid_digest};
use sqlx::types::time::OffsetDateTime;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

impl PostgresBrowserSessionStore {
    /// Revokes exactly the signed user's SID through one worker transaction.
    ///
    /// The user row serializes same-user commands. The immutable ledger stores
    /// the final no-op or changed outcome before the identity event is appended,
    /// so a retry never needs to mutate a ledger row or emit a second event.
    ///
    /// # Errors
    ///
    /// Returns `Unauthenticated` when the signed user is absent,
    /// `IdempotencyConflict` when a key is rebound to another SID or user, and
    /// `Unavailable` for persistence or event-commit failures.
    pub async fn revoke_browser_session(
        &self,
        command: RevokeBrowserSession,
    ) -> Result<RevokedBrowserSession, RevokeBrowserSessionError> {
        let mut transaction = self
            .worker_pool
            .begin()
            .await
            .map_err(|_| RevokeBrowserSessionError::Unavailable)?;
        let user_id = command.user_id;
        let user_status = lock_revocation_user(&mut transaction, user_id).await?;
        let idempotency_id =
            actor_idempotency_id(user_id.as_uuid().as_bytes(), &command.idempotency_seed);
        let sid_digest = browser_session_sid_digest(command.sid).as_bytes().to_vec();

        if let Some(replay) =
            replay_revocation(&mut transaction, idempotency_id, user_id, &sid_digest).await?
        {
            transaction
                .commit()
                .await
                .map_err(|_| RevokeBrowserSessionError::Unavailable)?;
            return Ok(replay);
        }

        let session = sqlx::query_as::<_, RevocationSessionRow>(
            "SELECT id, issued_at, expires_at, revoked_at
             FROM human_browser_sessions
             WHERE user_id = $1 AND sid_digest = $2
             FOR UPDATE",
        )
        .bind(user_id.as_uuid())
        .bind(&sid_digest)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| RevokeBrowserSessionError::Unavailable)?;
        // Obtain time after the row lock: transaction-start time can be stale
        // after waiting behind concurrent creation or revocation.
        let now: OffsetDateTime = sqlx::query_scalar("SELECT statement_timestamp()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| RevokeBrowserSessionError::Unavailable)?;
        let matched_session_id = session.as_ref().map(|row| row.id);
        let changed = session.as_ref().is_some_and(|row| {
            row.revoked_at.is_none() && row.issued_at <= now && now < row.expires_at
        });
        if changed {
            let Some(session_id) = matched_session_id else {
                return Err(RevokeBrowserSessionError::Unavailable);
            };
            sqlx::query(
                "UPDATE human_browser_sessions
                 SET revoked_at = $2, revocation_reason = 'logout'
                 WHERE id = $1 AND revoked_at IS NULL",
            )
            .bind(session_id)
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(|_| RevokeBrowserSessionError::Unavailable)?;
        }

        sqlx::query(
            "INSERT INTO human_browser_session_revocations
                 (revocation_idempotency_id, user_id, sid_digest, request_id,
                  matched_session_id, changed)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(idempotency_id.as_uuid())
        .bind(user_id.as_uuid())
        .bind(&sid_digest)
        .bind(command.request_id.as_uuid())
        .bind(matched_session_id)
        .bind(changed)
        .execute(&mut *transaction)
        .await
        .map_err(|_| RevokeBrowserSessionError::Unavailable)?;
        set_revocation_event_context(
            &mut transaction,
            user_id,
            command.request_id,
            idempotency_id,
        )
        .await?;
        append_revocation_identity_event(
            &mut transaction,
            user_id.as_uuid(),
            idempotency_id,
            &user_status,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|_| RevokeBrowserSessionError::Unavailable)?;
        Ok(RevokedBrowserSession {
            idempotency_id,
            changed,
        })
    }
}

async fn lock_revocation_user(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: UserId,
) -> Result<String, RevokeBrowserSessionError> {
    sqlx::query_scalar::<_, String>("SELECT status FROM users WHERE id = $1 FOR UPDATE")
        .bind(user_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| RevokeBrowserSessionError::Unavailable)?
        .ok_or(RevokeBrowserSessionError::Unauthenticated)
}

async fn replay_revocation(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_id: identity_domain::RequestId,
    user_id: UserId,
    sid_digest: &[u8],
) -> Result<Option<RevokedBrowserSession>, RevokeBrowserSessionError> {
    let Some(prior) = sqlx::query_as::<_, RevocationCommandRow>(
        "SELECT user_id, sid_digest, changed
         FROM human_browser_session_revocations
         WHERE revocation_idempotency_id = $1",
    )
    .bind(idempotency_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| RevokeBrowserSessionError::Unavailable)?
    else {
        return Ok(None);
    };
    if prior.user_id != user_id.as_uuid() || prior.sid_digest != sid_digest {
        return Err(RevokeBrowserSessionError::IdempotencyConflict);
    }
    Ok(Some(RevokedBrowserSession {
        idempotency_id,
        changed: prior.changed,
    }))
}

async fn set_revocation_event_context(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: UserId,
    request_id: identity_domain::RequestId,
    idempotency_id: identity_domain::RequestId,
) -> Result<(), RevokeBrowserSessionError> {
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
    .map_err(|_| RevokeBrowserSessionError::Unavailable)
}

async fn append_revocation_identity_event(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    idempotency_id: identity_domain::RequestId,
    user_status: &str,
) -> Result<(), RevokeBrowserSessionError> {
    sqlx::query(
        "SELECT event_id, cursor, aggregate_version
         FROM append_application_event(
             $1, 'identity', $2, 'identity_profile', $2,
             'identity.profile_changed', 'updated', $3, NULL, NULL
         )",
    )
    .bind(idempotency_id.as_uuid())
    .bind(user_id)
    .bind(user_status)
    .fetch_one(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|_| RevokeBrowserSessionError::Unavailable)
}

#[derive(FromRow)]
struct RevocationCommandRow {
    user_id: Uuid,
    sid_digest: Vec<u8>,
    changed: bool,
}

#[derive(FromRow)]
struct RevocationSessionRow {
    id: Uuid,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
}

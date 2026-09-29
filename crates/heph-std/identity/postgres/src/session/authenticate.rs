use super::PostgresBrowserSessionStore;
use identity_application::BrowserSessionAuthenticationError;
use identity_domain::{
    BrowserSessionId, BrowserSessionMetadata, BrowserSessionSid, UserId, browser_session_sid_digest,
};
use sqlx::FromRow;
use uuid::Uuid;

impl PostgresBrowserSessionStore {
    /// Authenticates one active session through the application-role verifier.
    ///
    /// The raw SID is reduced to the domain-separated digest before it crosses
    /// the adapter boundary. A missing row is an unauthenticated session; a
    /// query or metadata invariant failure is opaque provider unavailability.
    /// No worker query or signature-only fallback is permitted.
    ///
    /// # Errors
    ///
    /// Returns `Unauthenticated` when the SID, user, account state, or
    /// session lifetime does not match an active row. Returns `Unavailable`
    /// for application-role database failures or invalid returned metadata.
    pub async fn authenticate_browser_session(
        &self,
        user_id: UserId,
        sid: BrowserSessionSid,
    ) -> Result<BrowserSessionMetadata, BrowserSessionAuthenticationError> {
        let sid_digest = browser_session_sid_digest(sid).as_bytes().to_vec();
        let row = sqlx::query_as::<_, AuthenticatedSessionRow>(
            "SELECT session_id, user_id, issued_at, expires_at
             FROM authenticate_human_browser_session($1, $2)",
        )
        .bind(sid_digest)
        .bind(user_id.as_uuid())
        .fetch_optional(&self.application_pool)
        .await
        .map_err(|_| BrowserSessionAuthenticationError::Unavailable)?
        .ok_or(BrowserSessionAuthenticationError::Unauthenticated)?;
        BrowserSessionMetadata::new(
            BrowserSessionId::from_uuid(row.session_id),
            UserId::from_uuid(row.user_id),
            row.issued_at,
            row.expires_at,
            None,
        )
        .ok_or(BrowserSessionAuthenticationError::Unavailable)
    }
}

#[derive(FromRow)]
struct AuthenticatedSessionRow {
    session_id: Uuid,
    user_id: Uuid,
    issued_at: sqlx::types::time::OffsetDateTime,
    expires_at: sqlx::types::time::OffsetDateTime,
}

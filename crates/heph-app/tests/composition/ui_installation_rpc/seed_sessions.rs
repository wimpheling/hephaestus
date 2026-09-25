use super::ui_installation_seed::SeedData;
use identity_domain::{
    AuthenticatedIdentity, RequestId, UserId, browser_session_identity_binding_digest,
    browser_session_sid_digest,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) async fn seed_session_rows(pool: &PgPool, data: &SeedData) {
    sqlx::query(
        "INSERT INTO human_browser_sessions
             (id, sid_digest, creation_idempotency_id, creation_request_id,
              identity_binding_digest, user_id, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(data.parent_session_id)
    .bind(browser_session_sid_digest(data.sid).as_bytes().to_vec())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(
        browser_session_identity_binding_digest(&data.identity)
            .as_bytes()
            .to_vec(),
    )
    .bind(data.user_id)
    .bind(data.issued_at)
    .bind(data.issued_at + time::Duration::hours(1))
    .execute(pool)
    .await
    .expect("seed active parent browser session");
    let second_identity = AuthenticatedIdentity::new(
        UserId::from_uuid(data.second_user_id),
        data.issuer.clone(),
        String::from("ui-rpc-second-admin"),
        json!({"email_verified": true}),
        RequestId::new(),
    );
    sqlx::query(
        "INSERT INTO human_browser_sessions
             (id, sid_digest, creation_idempotency_id, creation_request_id,
              identity_binding_digest, user_id, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(data.second_parent_session_id)
    .bind(
        browser_session_sid_digest(data.second_sid)
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(
        browser_session_identity_binding_digest(&second_identity)
            .as_bytes()
            .to_vec(),
    )
    .bind(data.second_user_id)
    .bind(data.issued_at)
    .bind(data.issued_at + time::Duration::hours(1))
    .execute(pool)
    .await
    .expect("seed second active parent browser session");
}

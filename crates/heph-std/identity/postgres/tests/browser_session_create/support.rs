use identity_application::{CreateBrowserSession, VerifiedBrowserIdentity};
use identity_domain::{
    RequestId, UserId, actor_idempotency_id, browser_session_identity_binding_digest,
    browser_session_sid_digest,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub fn make_command(
    issuer: &str,
    subject: &str,
    sid: identity_domain::BrowserSessionSid,
    request_id: RequestId,
    seed: u8,
) -> CreateBrowserSession {
    CreateBrowserSession {
        request_id,
        idempotency_seed: [seed; 32],
        verified: VerifiedBrowserIdentity {
            issuer: issuer.to_owned(),
            subject: subject.to_owned(),
        },
        sid,
    }
}

pub async fn worker_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(database_url)
        .await
        .expect("connect worker role")
}

pub async fn application_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(database_url)
        .await
        .expect("connect application role")
}

pub async fn insert_identity(pool: &PgPool, issuer: &str, subject: &str) -> UserId {
    let user_id: Uuid = sqlx::query_scalar("SELECT gen_random_uuid()")
        .fetch_one(pool)
        .await
        .expect("generate session user id");
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id)
        .bind(format!("browser session {user_id}"))
        .execute(pool)
        .await
        .expect("insert session user");
    sqlx::query(
        "INSERT INTO external_identities (user_id, issuer, subject, provider_metadata)
         VALUES ($1, $2, $3, '{}'::jsonb)",
    )
    .bind(user_id)
    .bind(issuer)
    .bind(subject)
    .execute(pool)
    .await
    .expect("insert external identity");
    UserId::from_uuid(user_id)
}

pub async fn insert_identity_mapping(pool: &PgPool, user_id: UserId, issuer: &str, subject: &str) {
    sqlx::query(
        "INSERT INTO external_identities (user_id, issuer, subject, provider_metadata)
         VALUES ($1, $2, $3, '{}'::jsonb)",
    )
    .bind(user_id.as_uuid())
    .bind(issuer)
    .bind(subject)
    .execute(pool)
    .await
    .expect("insert external identity mapping");
}

pub async fn insert_expired_session(
    pool: &PgPool,
    command: &CreateBrowserSession,
    user_id: UserId,
) {
    let idempotency_id =
        actor_idempotency_id(user_id.as_uuid().as_bytes(), &command.idempotency_seed);
    let verified = identity_domain::AuthenticatedIdentity::new(
        user_id,
        &command.verified.issuer,
        &command.verified.subject,
        serde_json::Value::Null,
        command.request_id,
    );
    let identity_digest = browser_session_identity_binding_digest(&verified)
        .as_bytes()
        .to_vec();
    let sid_digest = browser_session_sid_digest(command.sid).as_bytes().to_vec();
    sqlx::query(
        "INSERT INTO human_browser_sessions (
             id, sid_digest, creation_idempotency_id, creation_request_id,
             identity_binding_digest, user_id, issued_at, expires_at
         ) VALUES ($1, $2, $3, $4, $5, $6,
                   now() - interval '2 hours', now() - interval '1 second')",
    )
    .bind(Uuid::new_v4())
    .bind(sid_digest)
    .bind(idempotency_id.as_uuid())
    .bind(command.request_id.as_uuid())
    .bind(identity_digest)
    .bind(user_id.as_uuid())
    .execute(pool)
    .await
    .expect("insert expired session fixture");
}

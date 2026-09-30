use event_application::MutationReceiptReader;
use event_postgres::PostgresMutationReceiptReader;
use identity_application::{BrowserSessionStore, RevokeBrowserSession, RevokeBrowserSessionError};
use identity_domain::{BrowserSessionSid, RequestId, UserId, browser_session_sid_digest};
use identity_postgres::PostgresBrowserSessionStore;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub const ACTIVE: &str = "active";
pub const SUSPENDED: &str = "suspended";
pub const DISABLED: &str = "disabled";
pub const EXPIRED: &str = "expired";
pub const FUTURE: &str = "future";

pub async fn revoke(
    store: &PostgresBrowserSessionStore,
    command: RevokeBrowserSession,
) -> Result<identity_application::RevokedBrowserSession, RevokeBrowserSessionError> {
    BrowserSessionStore::revoke_browser_session(store, command).await
}

pub fn command(
    user_id: UserId,
    sid: BrowserSessionSid,
    seed: u8,
    _request: u8,
) -> RevokeBrowserSession {
    command_with_request(user_id, sid, seed, RequestId::new())
}

pub const fn command_with_request(
    user_id: UserId,
    sid: BrowserSessionSid,
    seed: u8,
    request_id: RequestId,
) -> RevokeBrowserSession {
    RevokeBrowserSession {
        request_id,
        idempotency_seed: [seed; 32],
        user_id,
        sid,
    }
}

pub fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut digest = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut digest, "{byte:02x}").expect("writing a digest to String cannot fail");
    }
    digest
}

pub async fn worker_pool(database_url: &str) -> PgPool {
    role_pool(database_url, "hephaestus_worker").await
}

pub async fn application_pool(database_url: &str) -> PgPool {
    role_pool(database_url, "hephaestus_app").await
}

async fn role_pool(database_url: &str, role: &str) -> PgPool {
    let worker = role == "hephaestus_worker";
    PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _metadata| {
            Box::pin(async move {
                if worker {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await
                        .map(|_| ())
                } else {
                    sqlx::query("SET ROLE hephaestus_app")
                        .execute(connection)
                        .await
                        .map(|_| ())
                }
            })
        })
        .connect(database_url)
        .await
        .expect("connect session role")
}

pub async fn insert_user(pool: &PgPool, user_id: UserId, status: &str) {
    sqlx::query("INSERT INTO users (id, display_name, status) VALUES ($1, $2, $3)")
        .bind(user_id.as_uuid())
        .bind(format!("browser session {user_id}"))
        .bind(status)
        .execute(pool)
        .await
        .expect("insert session user");
}

pub async fn insert_session(
    pool: &PgPool,
    user_id: UserId,
    sid: BrowserSessionSid,
    kind: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let query = match kind {
        ACTIVE => sqlx::query(
            "INSERT INTO human_browser_sessions (id, sid_digest, creation_idempotency_id, creation_request_id, identity_binding_digest, user_id, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, now() - interval '1 hour', now() + interval '11 hours')",
        ),
        EXPIRED => sqlx::query(
            "INSERT INTO human_browser_sessions (id, sid_digest, creation_idempotency_id, creation_request_id, identity_binding_digest, user_id, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, now() - interval '2 hours', now() - interval '1 hour')",
        ),
        FUTURE => sqlx::query(
            "INSERT INTO human_browser_sessions (id, sid_digest, creation_idempotency_id, creation_request_id, identity_binding_digest, user_id, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, now() + interval '1 hour', now() + interval '2 hours')",
        ),
        _ => panic!("unsupported session fixture kind"),
    };
    query
        .bind(id)
        .bind(browser_session_sid_digest(sid).as_bytes().to_vec())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(vec![0x44_u8; 32])
        .bind(user_id.as_uuid())
        .execute(pool)
        .await
        .expect("insert session fixture");
    id
}

pub async fn assert_receipt(
    reader: &PostgresMutationReceiptReader,
    occurrence_id: RequestId,
    user_id: UserId,
) {
    let receipt = reader
        .load(occurrence_id, user_id, "identity_profile", "identity")
        .await
        .expect("load typed identity mutation receipt");
    assert_eq!(receipt.scope_kind, "identity");
    assert_eq!(receipt.scope_id, user_id.as_uuid());
    assert!(receipt.cursor > 0);
    assert!(receipt.aggregate_version > 0);
}

pub async fn assert_event_and_outbox(pool: &PgPool, occurrence_id: RequestId) {
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE occurrence_id = $1 AND event_type = 'identity.profile_changed'",
    )
    .bind(occurrence_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("count identity event");
    assert_eq!(event_count, 1);
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM product_event_outbox AS outbox
         JOIN application_events AS event ON event.id = outbox.event_id
         WHERE event.occurrence_id = $1",
    )
    .bind(occurrence_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("count identity event outbox");
    assert_eq!(outbox_count, 1);
}

pub async fn ledger_count(pool: &PgPool, idempotency_id: RequestId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM human_browser_session_revocations
         WHERE revocation_idempotency_id = $1",
    )
    .bind(idempotency_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("count revocation ledger row")
}

pub async fn ledger_request_id(pool: &PgPool, idempotency_id: RequestId) -> Uuid {
    sqlx::query_scalar(
        "SELECT request_id FROM human_browser_session_revocations
         WHERE revocation_idempotency_id = $1",
    )
    .bind(idempotency_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read original revocation request")
}

pub async fn event_state(pool: &PgPool, occurrence_id: RequestId) -> Option<String> {
    sqlx::query_scalar("SELECT safe_state FROM application_events WHERE occurrence_id = $1")
        .bind(occurrence_id.as_uuid())
        .fetch_optional(pool)
        .await
        .expect("read identity event state")
}

pub async fn event_state_for_user(pool: &PgPool, user_id: UserId) -> Option<String> {
    sqlx::query_scalar(
        "SELECT safe_state FROM application_events
         WHERE aggregate_type = 'identity_profile' AND aggregate_id = $1
         ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(user_id.as_uuid())
    .fetch_optional(pool)
    .await
    .expect("read user identity event state")
}

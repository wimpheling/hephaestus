//! Real-role coverage for browser-session self-revocation.

use event_application::MutationReceiptReader;
use event_postgres::PostgresMutationReceiptReader;
use identity_application::{BrowserSessionStore, RevokeBrowserSession, RevokeBrowserSessionError};
use identity_domain::{BrowserSessionSid, RequestId, UserId, browser_session_sid_digest};
use identity_postgres::PostgresBrowserSessionStore;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const ACTIVE: &str = "active";
const SUSPENDED: &str = "suspended";
const DISABLED: &str = "disabled";
const EXPIRED: &str = "expired";
const FUTURE: &str = "future";

#[tokio::test]
#[serial]
// This one real-role test keeps the complete revocation matrix together so
// every outcome is checked through the typed receipt reader and durable ledger.
#[allow(clippy::too_many_lines)]
async fn browser_session_revocation_is_idempotent_and_actor_bound() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!(
            "skipping browser session revocation test: HEPHAESTUS_POSTGRES_TEST_URL is unset"
        );
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect migration database");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through browser session revocation");
    bootstrap.close().await;

    let worker = worker_pool(&database_url).await;
    let application = application_pool(&database_url).await;
    let store = PostgresBrowserSessionStore::new(worker.clone(), application);
    let receipts = PostgresMutationReceiptReader::new(worker.clone());
    let active_user = UserId::from_uuid(Uuid::new_v4());
    let suspended_user = UserId::from_uuid(Uuid::new_v4());
    let disabled_user = UserId::from_uuid(Uuid::new_v4());
    let other_user = UserId::from_uuid(Uuid::new_v4());
    for (user_id, status) in [
        (active_user, ACTIVE),
        (suspended_user, SUSPENDED),
        (disabled_user, DISABLED),
        (other_user, ACTIVE),
    ] {
        insert_user(&worker, user_id, status).await;
    }

    let active_sid = BrowserSessionSid::new();
    insert_session(&worker, active_user, active_sid, ACTIVE).await;
    let before_authentication =
        BrowserSessionStore::authenticate_browser_session(&store, active_user, active_sid)
            .await
            .expect("active verifier accepts the live session");
    assert_eq!(before_authentication.user_id(), active_user);
    let first_request_id = RequestId::new();
    let first = revoke(
        &store,
        command_with_request(active_user, active_sid, 1, first_request_id),
    )
    .await
    .expect("revoke active session");
    assert!(first.changed);
    assert_receipt(&receipts, first.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, first.idempotency_id).await;
    assert_eq!(ledger_count(&worker, first.idempotency_id).await, 1);
    assert_eq!(
        ledger_request_id(&worker, first.idempotency_id).await,
        first_request_id.as_uuid()
    );
    let revoked_at: Option<sqlx::types::time::OffsetDateTime> =
        sqlx::query_scalar("SELECT revoked_at FROM human_browser_sessions WHERE sid_digest = $1")
            .bind(browser_session_sid_digest(active_sid).as_bytes().to_vec())
            .fetch_one(&worker)
            .await
            .expect("read persisted revocation timestamp");
    assert!(revoked_at.is_some());
    let revocation_reason: Option<String> = sqlx::query_scalar(
        "SELECT revocation_reason FROM human_browser_sessions WHERE sid_digest = $1",
    )
    .bind(browser_session_sid_digest(active_sid).as_bytes().to_vec())
    .fetch_one(&worker)
    .await
    .expect("read persisted revocation reason");
    assert_eq!(revocation_reason.as_deref(), Some("logout"));
    assert!(matches!(
        BrowserSessionStore::authenticate_browser_session(&store, active_user, active_sid).await,
        Err(identity_application::BrowserSessionAuthenticationError::Unauthenticated)
    ));
    assert_eq!(
        event_state(&worker, first.idempotency_id).await,
        Some("active".into())
    );
    let event_json: String = sqlx::query_scalar(
        "SELECT row_to_json(event)::text
         FROM application_events AS event WHERE event.occurrence_id = $1",
    )
    .bind(first.idempotency_id.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("read redacted revocation event");
    assert!(!event_json.contains(&active_sid.to_protocol_string()));
    assert!(!event_json.contains(&hex_digest(
        &browser_session_sid_digest(active_sid).as_bytes()
    )));

    let replay = revoke(&store, command(active_user, active_sid, 1, 2))
        .await
        .expect("replay active revocation");
    assert_eq!(replay, first);
    assert_receipt(&receipts, replay.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, replay.idempotency_id).await;
    assert_eq!(ledger_count(&worker, replay.idempotency_id).await, 1);
    assert_eq!(
        ledger_request_id(&worker, replay.idempotency_id).await,
        first_request_id.as_uuid()
    );

    let new_key_noop = revoke(&store, command(active_user, active_sid, 2, 3))
        .await
        .expect("new key on already-revoked session is a no-op");
    assert!(!new_key_noop.changed);
    assert_receipt(&receipts, new_key_noop.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, new_key_noop.idempotency_id).await;
    assert_eq!(ledger_count(&worker, new_key_noop.idempotency_id).await, 1);

    let absent_sid = BrowserSessionSid::new();
    let absent_request_id = RequestId::new();
    let absent = revoke(
        &store,
        command_with_request(active_user, absent_sid, 3, absent_request_id),
    )
    .await
    .expect("absent SID is a durable no-op");
    assert!(!absent.changed);
    assert_receipt(&receipts, absent.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, absent.idempotency_id).await;
    assert_eq!(ledger_count(&worker, absent.idempotency_id).await, 1);
    let absent_replay = revoke(&store, command(active_user, absent_sid, 3, 40))
        .await
        .expect("replay absent-SID no-op");
    assert_eq!(absent_replay, absent);
    assert_receipt(&receipts, absent.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, absent.idempotency_id).await;
    assert_eq!(ledger_count(&worker, absent.idempotency_id).await, 1);
    assert_eq!(
        ledger_request_id(&worker, absent.idempotency_id).await,
        absent_request_id.as_uuid()
    );

    let expired_sid = BrowserSessionSid::new();
    insert_session(&worker, active_user, expired_sid, EXPIRED).await;
    let expired = revoke(&store, command(active_user, expired_sid, 4, 5))
        .await
        .expect("expired SID is a no-op");
    assert!(!expired.changed);
    assert_receipt(&receipts, expired.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, expired.idempotency_id).await;
    let future_sid = BrowserSessionSid::new();
    insert_session(&worker, active_user, future_sid, FUTURE).await;
    let future = revoke(&store, command(active_user, future_sid, 5, 6))
        .await
        .expect("future SID is a no-op");
    assert!(!future.changed);
    assert_receipt(&receipts, future.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, future.idempotency_id).await;

    let suspended_sid = BrowserSessionSid::new();
    insert_session(&worker, suspended_user, suspended_sid, ACTIVE).await;
    let suspended = revoke(&store, command(suspended_user, suspended_sid, 6, 7))
        .await
        .expect("suspended user may revoke its session");
    assert!(suspended.changed);
    assert_receipt(&receipts, suspended.idempotency_id, suspended_user).await;
    assert_event_and_outbox(&worker, suspended.idempotency_id).await;
    assert_eq!(
        event_state_for_user(&worker, suspended_user).await,
        Some("disabled".into())
    );

    let disabled_sid = BrowserSessionSid::new();
    insert_session(&worker, disabled_user, disabled_sid, ACTIVE).await;
    let disabled = revoke(&store, command(disabled_user, disabled_sid, 7, 8))
        .await
        .expect("disabled user may revoke its session");
    assert!(disabled.changed);
    assert_receipt(&receipts, disabled.idempotency_id, disabled_user).await;
    assert_event_and_outbox(&worker, disabled.idempotency_id).await;

    let wrong_user_sid = BrowserSessionSid::new();
    let wrong_user_session = insert_session(&worker, active_user, wrong_user_sid, ACTIVE).await;
    let wrong_user = revoke(&store, command(other_user, wrong_user_sid, 8, 9))
        .await
        .expect("cross-user digest is a no-op");
    assert!(!wrong_user.changed);
    assert_receipt(&receipts, wrong_user.idempotency_id, other_user).await;
    assert_event_and_outbox(&worker, wrong_user.idempotency_id).await;
    let still_active: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM human_browser_sessions
         WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(wrong_user_session)
    .fetch_optional(&worker)
    .await
    .expect("check cross-user session");
    assert_eq!(still_active, Some(wrong_user_session));

    let missing_user = UserId::from_uuid(Uuid::new_v4());
    assert!(matches!(
        revoke(
            &store,
            command(missing_user, BrowserSessionSid::new(), 9, 10)
        )
        .await,
        Err(RevokeBrowserSessionError::Unauthenticated)
    ));

    let conflict_sid = BrowserSessionSid::new();
    insert_session(&worker, active_user, conflict_sid, ACTIVE).await;
    let conflict = revoke(&store, command(active_user, conflict_sid, 10, 11))
        .await
        .expect("claim conflict key");
    assert!(conflict.changed);
    assert_receipt(&receipts, conflict.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, conflict.idempotency_id).await;
    assert!(matches!(
        revoke(
            &store,
            command(active_user, BrowserSessionSid::new(), 10, 12)
        )
        .await,
        Err(RevokeBrowserSessionError::IdempotencyConflict)
    ));

    let concurrent_sid = BrowserSessionSid::new();
    insert_session(&worker, active_user, concurrent_sid, ACTIVE).await;
    let left_store = store.clone();
    let right_store = store.clone();
    let (left, right) = tokio::join!(
        revoke(&left_store, command(active_user, concurrent_sid, 11, 13)),
        revoke(&right_store, command(active_user, concurrent_sid, 11, 14)),
    );
    let left = left.expect("first concurrent revocation");
    let right = right.expect("replayed concurrent revocation");
    assert_eq!(left, right);
    assert!(left.changed);
    assert_receipt(&receipts, left.idempotency_id, active_user).await;
    assert_event_and_outbox(&worker, left.idempotency_id).await;
    assert_eq!(ledger_count(&worker, left.idempotency_id).await, 1);

    println!(
        "REAL_BROWSER_SESSION_REVOKE=1 active=revoke replay=new-key-noop absent=expired=future-noop inactive-user=allowed wrong-user=unchanged conflict=typed concurrent=one-event"
    );
}

async fn revoke(
    store: &PostgresBrowserSessionStore,
    command: RevokeBrowserSession,
) -> Result<identity_application::RevokedBrowserSession, RevokeBrowserSessionError> {
    BrowserSessionStore::revoke_browser_session(store, command).await
}

fn command(
    user_id: UserId,
    sid: BrowserSessionSid,
    seed: u8,
    _request: u8,
) -> RevokeBrowserSession {
    command_with_request(user_id, sid, seed, RequestId::new())
}

const fn command_with_request(
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

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut digest = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut digest, "{byte:02x}").expect("writing a digest to String cannot fail");
    }
    digest
}

async fn worker_pool(database_url: &str) -> PgPool {
    role_pool(database_url, "hephaestus_worker").await
}

async fn application_pool(database_url: &str) -> PgPool {
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

async fn insert_user(pool: &PgPool, user_id: UserId, status: &str) {
    sqlx::query("INSERT INTO users (id, display_name, status) VALUES ($1, $2, $3)")
        .bind(user_id.as_uuid())
        .bind(format!("browser session {user_id}"))
        .bind(status)
        .execute(pool)
        .await
        .expect("insert session user");
}

async fn insert_session(
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

async fn assert_receipt(
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

async fn assert_event_and_outbox(pool: &PgPool, occurrence_id: RequestId) {
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

async fn ledger_count(pool: &PgPool, idempotency_id: RequestId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM human_browser_session_revocations
         WHERE revocation_idempotency_id = $1",
    )
    .bind(idempotency_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("count revocation ledger row")
}

async fn ledger_request_id(pool: &PgPool, idempotency_id: RequestId) -> Uuid {
    sqlx::query_scalar(
        "SELECT request_id FROM human_browser_session_revocations
         WHERE revocation_idempotency_id = $1",
    )
    .bind(idempotency_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read original revocation request")
}

async fn event_state(pool: &PgPool, occurrence_id: RequestId) -> Option<String> {
    sqlx::query_scalar("SELECT safe_state FROM application_events WHERE occurrence_id = $1")
        .bind(occurrence_id.as_uuid())
        .fetch_optional(pool)
        .await
        .expect("read identity event state")
}

async fn event_state_for_user(pool: &PgPool, user_id: UserId) -> Option<String> {
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

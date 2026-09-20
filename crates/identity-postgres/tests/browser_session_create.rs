//! Real-worker coverage for browser-session creation and replay.

use identity_application::{
    CreateBrowserSession, CreateBrowserSessionError, VerifiedBrowserIdentity,
};
use identity_domain::{
    DEFAULT_BROWSER_SESSION_TTL_SECONDS, RequestId, UserId, actor_idempotency_id,
    browser_session_identity_binding_digest, browser_session_sid_digest,
};
use identity_postgres::PostgresBrowserSessionStore;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::fmt::Write as _;
use uuid::Uuid;

#[tokio::test]
#[serial]
// This single real-database scenario keeps the idempotency, lifecycle,
// concurrency, and event-redaction assertions together so they share one
// worker-role fixture and cannot silently pass with a skipped database.
#[allow(clippy::too_many_lines)]
async fn worker_creation_replay_conflicts_and_concurrency_are_atomic() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping browser session creation test: HEPHAESTUS_POSTGRES_TEST_URL is unset");
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
        .expect("apply migrations through browser sessions");
    bootstrap.close().await;
    let pool = worker_pool(&database_url).await;
    let application_pool = application_pool(&database_url).await;
    let store = PostgresBrowserSessionStore::new(pool.clone(), application_pool);

    let issuer = format!("https://issuer-{}.example", Uuid::new_v4());
    let subject = format!("subject-{}", Uuid::new_v4());
    let user_id = insert_identity(&pool, &issuer, &subject).await;
    let sid = identity_domain::BrowserSessionSid::new();
    let request_id = RequestId::new();
    let command = make_command(&issuer, &subject, sid, request_id, 1);
    let created = store
        .create_browser_session(command)
        .await
        .expect("create active session");
    let replay_request_id = RequestId::new();
    let replay = store
        .create_browser_session(make_command(&issuer, &subject, sid, replay_request_id, 1))
        .await
        .expect("replay active session");
    assert_eq!(created, replay);
    assert_eq!(created.metadata.user_id(), user_id);
    let session_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM human_browser_sessions
         WHERE creation_idempotency_id = $1",
    )
    .bind(created.idempotency_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count created session");
    assert_eq!(session_count, 1);
    let stored_creation_request_id: Uuid = sqlx::query_scalar(
        "SELECT creation_request_id
         FROM human_browser_sessions WHERE id = $1",
    )
    .bind(created.metadata.id().as_uuid())
    .fetch_one(&pool)
    .await
    .expect("read original creation request");
    assert_eq!(stored_creation_request_id, request_id.as_uuid());
    let event: (String, String, Option<Uuid>, Option<Uuid>, String, String) = sqlx::query_as(
        "SELECT event.safe_state, event.change_kind, event.related_id_one,
                    event.related_id_two, event.aggregate_type, event.event_type
             FROM application_events AS event
             WHERE event.occurrence_id = $1",
    )
    .bind(created.idempotency_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("read creation event");
    assert_eq!(
        event,
        (
            String::from("active"),
            String::from("updated"),
            None,
            None,
            String::from("identity_profile"),
            String::from("identity.profile_changed")
        )
    );
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM product_event_outbox AS outbox
         JOIN application_events AS event ON event.id = outbox.event_id
         WHERE event.occurrence_id = $1",
    )
    .bind(created.idempotency_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count creation outbox");
    assert_eq!(outbox_count, 1);
    let event_json: String = sqlx::query_scalar(
        "SELECT row_to_json(event)::text
         FROM application_events AS event
         WHERE event.occurrence_id = $1",
    )
    .bind(created.idempotency_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("serialize creation event");
    assert!(!event_json.contains(&sid.to_protocol_string()));
    assert!(!event_json.contains(&issuer));
    assert!(!event_json.contains(&subject));
    let mut sid_digest_hex = String::with_capacity(64);
    for byte in browser_session_sid_digest(sid).as_bytes() {
        write!(&mut sid_digest_hex, "{byte:02x}").expect("write digest to string");
    }
    assert!(!event_json.contains(&sid_digest_hex));
    let lifetime_seconds: i64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM (expires_at - issued_at))::bigint
         FROM human_browser_sessions WHERE id = $1",
    )
    .bind(created.metadata.id().as_uuid())
    .fetch_one(&pool)
    .await
    .expect("read session lifetime");
    assert_eq!(lifetime_seconds, DEFAULT_BROWSER_SESSION_TTL_SECONDS);

    let mismatch_sid = identity_domain::BrowserSessionSid::new();
    assert!(matches!(
        store
            .create_browser_session(make_command(&issuer, &subject, mismatch_sid, request_id, 1))
            .await,
        Err(CreateBrowserSessionError::IdempotencyConflict)
    ));
    let second_issuer = format!("https://issuer-{}.example", Uuid::new_v4());
    let second_subject = format!("subject-{}", Uuid::new_v4());
    insert_identity_mapping(&pool, user_id, &second_issuer, &second_subject).await;
    let identity_mismatch_sid = identity_domain::BrowserSessionSid::new();
    assert!(matches!(
        store
            .create_browser_session(make_command(
                &second_issuer,
                &second_subject,
                identity_mismatch_sid,
                RequestId::new(),
                1,
            ))
            .await,
        Err(CreateBrowserSessionError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .create_browser_session(make_command(&issuer, &subject, sid, RequestId::new(), 2))
            .await,
        Err(CreateBrowserSessionError::IdempotencyConflict)
    ));
    assert!(
        !format!("{:?}", CreateBrowserSessionError::IdempotencyConflict)
            .contains(&sid.to_protocol_string())
    );

    let expired_issuer = format!("https://issuer-{}.example", Uuid::new_v4());
    let expired_subject = format!("subject-{}", Uuid::new_v4());
    let expired_user = insert_identity(&pool, &expired_issuer, &expired_subject).await;
    let expired_sid = identity_domain::BrowserSessionSid::new();
    let expired_command = make_command(
        &expired_issuer,
        &expired_subject,
        expired_sid,
        RequestId::new(),
        3,
    );
    insert_expired_session(&pool, &expired_command, expired_user).await;
    assert!(matches!(
        store.create_browser_session(expired_command).await,
        Err(CreateBrowserSessionError::InactiveReplay)
    ));

    let revoked_issuer = format!("https://issuer-{}.example", Uuid::new_v4());
    let revoked_subject = format!("subject-{}", Uuid::new_v4());
    insert_identity(&pool, &revoked_issuer, &revoked_subject).await;
    let revoked_sid = identity_domain::BrowserSessionSid::new();
    let revoked_command = make_command(
        &revoked_issuer,
        &revoked_subject,
        revoked_sid,
        RequestId::new(),
        4,
    );
    let revoked_request_id = revoked_command.request_id;
    let revoked = store
        .create_browser_session(revoked_command)
        .await
        .expect("create session before revoke");
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
         WHERE id = $1",
    )
    .bind(revoked.metadata.id().as_uuid())
    .execute(&pool)
    .await
    .expect("revoke fixture session");
    assert!(matches!(
        store
            .create_browser_session(make_command(
                &revoked_issuer,
                &revoked_subject,
                revoked_sid,
                revoked_request_id,
                4,
            ))
            .await,
        Err(CreateBrowserSessionError::InactiveReplay)
    ));

    let missing = make_command(
        "https://unmapped.example",
        "unmapped-subject",
        identity_domain::BrowserSessionSid::new(),
        RequestId::new(),
        5,
    );
    assert!(matches!(
        store.create_browser_session(missing).await,
        Err(CreateBrowserSessionError::PermissionDenied)
    ));
    let inactive_issuer = format!("https://issuer-{}.example", Uuid::new_v4());
    let inactive_subject = format!("subject-{}", Uuid::new_v4());
    let inactive_user = insert_identity(&pool, &inactive_issuer, &inactive_subject).await;
    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(inactive_user.as_uuid())
        .execute(&pool)
        .await
        .expect("suspend user fixture");
    assert!(matches!(
        store
            .create_browser_session(make_command(
                &inactive_issuer,
                &inactive_subject,
                identity_domain::BrowserSessionSid::new(),
                RequestId::new(),
                6,
            ))
            .await,
        Err(CreateBrowserSessionError::PermissionDenied)
    ));

    let concurrent_issuer = format!("https://issuer-{}.example", Uuid::new_v4());
    let concurrent_subject = format!("subject-{}", Uuid::new_v4());
    insert_identity(&pool, &concurrent_issuer, &concurrent_subject).await;
    let concurrent_sid = identity_domain::BrowserSessionSid::new();
    let left_store = store.clone();
    let right_store = store.clone();
    let left_command = make_command(
        &concurrent_issuer,
        &concurrent_subject,
        concurrent_sid,
        RequestId::new(),
        7,
    );
    let right_command = make_command(
        &concurrent_issuer,
        &concurrent_subject,
        concurrent_sid,
        left_command.request_id,
        7,
    );
    let (left, right) = tokio::join!(
        left_store.create_browser_session(left_command),
        right_store.create_browser_session(right_command),
    );
    let left = left.expect("first concurrent creation");
    let right = right.expect("replayed concurrent creation");
    assert_eq!(left, right);
    let concurrent_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE occurrence_id = $1")
            .bind(left.idempotency_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count concurrent events");
    assert_eq!(concurrent_events, 1);
    println!(
        "REAL_BROWSER_SESSION_CREATE=1 worker_role=hephaestus_worker replay=active conflicts=identity+sid+key inactive=expired+revoked concurrency=one_row_one_event"
    );
}

fn make_command(
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

async fn worker_pool(database_url: &str) -> PgPool {
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

async fn application_pool(database_url: &str) -> PgPool {
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

async fn insert_identity(pool: &PgPool, issuer: &str, subject: &str) -> UserId {
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

async fn insert_identity_mapping(pool: &PgPool, user_id: UserId, issuer: &str, subject: &str) {
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

async fn insert_expired_session(pool: &PgPool, command: &CreateBrowserSession, user_id: UserId) {
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

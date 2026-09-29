//! Real-worker coverage for browser-session revocation idempotency storage.

use identity_domain::{BrowserSessionSid, browser_session_sid_digest};
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 86;

#[tokio::test]
#[serial]
// This real-role scenario keeps the permission, binding, immutability, and
// constraint assertions together so the schema marker cannot pass partially.
#[allow(clippy::too_many_lines)]
async fn revocation_ledger_is_worker_owned_immutable_and_sid_bound() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!(
            "skipping browser session revocation schema test: HEPHAESTUS_POSTGRES_TEST_URL is unset"
        );
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0086");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker");
    assert!(
        max_migration >= EXPECTED_MIGRATION,
        "revocation ledger requires head >= {EXPECTED_MIGRATION}, got {max_migration}"
    );
    bootstrap.close().await;

    let worker = worker_pool(&database_url).await;
    let app = app_pool(&database_url).await;
    assert_role(&worker, "hephaestus_worker", false, true).await;
    assert_role(&app, "hephaestus_app", false, false).await;

    let user_id = Uuid::new_v4();
    let other_user_id = Uuid::new_v4();
    insert_user(&worker, user_id).await;
    insert_user(&worker, other_user_id).await;

    let sid = BrowserSessionSid::new();
    let sid_digest = browser_session_sid_digest(sid).as_bytes().to_vec();
    let session_id = insert_session(&worker, user_id, &sid_digest).await;
    let first_key = Uuid::new_v4();
    insert_ledger(
        &worker,
        first_key,
        user_id,
        &sid_digest,
        Some(session_id),
        true,
    )
    .await;
    let stored: (Uuid, Uuid, Vec<u8>, Option<Uuid>, bool) = sqlx::query_as(
        "SELECT revocation_idempotency_id, user_id, sid_digest,
                matched_session_id, changed
         FROM human_browser_session_revocations
         WHERE revocation_idempotency_id = $1",
    )
    .bind(first_key)
    .fetch_one(&worker)
    .await
    .expect("worker reads revocation ledger");
    assert_eq!(
        stored,
        (
            first_key,
            user_id,
            sid_digest.clone(),
            Some(session_id),
            true
        )
    );

    // A later logout command for the same SID is an independent successful
    // no-op and therefore has its own durable receipt binding.
    let second_key = Uuid::new_v4();
    insert_ledger(
        &worker,
        second_key,
        user_id,
        &stored.2,
        Some(session_id),
        false,
    )
    .await;

    let absent_sid = browser_session_sid_digest(BrowserSessionSid::new())
        .as_bytes()
        .to_vec();
    let absent_key = Uuid::new_v4();
    insert_ledger(&worker, absent_key, user_id, &absent_sid, None, false).await;
    let absent_session: Option<Uuid> = sqlx::query_scalar(
        "SELECT matched_session_id
         FROM human_browser_session_revocations
         WHERE revocation_idempotency_id = $1",
    )
    .bind(absent_key)
    .fetch_one(&worker)
    .await
    .expect("read absent-session no-op ledger");
    assert_eq!(absent_session, None);

    let changed_without_match = sqlx::query(
        "INSERT INTO human_browser_session_revocations
             (revocation_idempotency_id, user_id, sid_digest, request_id,
              matched_session_id, changed)
         VALUES (gen_random_uuid(), $1, $2, gen_random_uuid(), NULL, true)",
    )
    .bind(user_id)
    .bind(
        browser_session_sid_digest(BrowserSessionSid::new())
            .as_bytes()
            .to_vec(),
    )
    .execute(&worker)
    .await
    .expect_err("changed revocation must name a matched session");
    assert_eq!(
        duplicate_key_code(&changed_without_match).as_deref(),
        Some("23514")
    );

    let duplicate_key = sqlx::query(
        "INSERT INTO human_browser_session_revocations
             (revocation_idempotency_id, user_id, sid_digest, request_id,
              matched_session_id, changed)
         VALUES ($1, $2, $3, gen_random_uuid(), $4, false)",
    )
    .bind(first_key)
    .bind(user_id)
    .bind(&sid_digest)
    .bind(session_id)
    .execute(&worker)
    .await
    .expect_err("same revocation key must be unique");
    assert_eq!(duplicate_key_code(&duplicate_key).as_deref(), Some("23505"));

    let mismatched_user_fk = sqlx::query(
        "INSERT INTO human_browser_session_revocations
             (revocation_idempotency_id, user_id, sid_digest, request_id,
              matched_session_id, changed)
         VALUES (gen_random_uuid(), $1, $2, gen_random_uuid(), $3, false)",
    )
    .bind(other_user_id)
    .bind(&sid_digest)
    .bind(session_id)
    .execute(&worker)
    .await
    .expect_err("matched session must belong to the ledger user");
    assert_eq!(
        duplicate_key_code(&mismatched_user_fk).as_deref(),
        Some("23503")
    );

    let mismatched_sid_fk = sqlx::query(
        "INSERT INTO human_browser_session_revocations
             (revocation_idempotency_id, user_id, sid_digest, request_id,
              matched_session_id, changed)
         VALUES (gen_random_uuid(), $1, $2, gen_random_uuid(), $3, false)",
    )
    .bind(user_id)
    .bind(
        browser_session_sid_digest(BrowserSessionSid::new())
            .as_bytes()
            .to_vec(),
    )
    .bind(session_id)
    .execute(&worker)
    .await
    .expect_err("matched session must use the ledger SID digest");
    assert_eq!(
        duplicate_key_code(&mismatched_sid_fk).as_deref(),
        Some("23503")
    );

    let update_denied = sqlx::query(
        "UPDATE human_browser_session_revocations
         SET changed = false WHERE revocation_idempotency_id = $1",
    )
    .bind(first_key)
    .execute(&worker)
    .await
    .expect_err("worker must not update immutable revocation ledger");
    assert_eq!(duplicate_key_code(&update_denied).as_deref(), Some("42501"));
    let delete_denied = sqlx::query(
        "DELETE FROM human_browser_session_revocations
         WHERE revocation_idempotency_id = $1",
    )
    .bind(first_key)
    .execute(&worker)
    .await
    .expect_err("worker must not delete immutable revocation ledger");
    assert_eq!(duplicate_key_code(&delete_denied).as_deref(), Some("42501"));

    let app_select = sqlx::query(
        "SELECT revocation_idempotency_id
         FROM human_browser_session_revocations LIMIT 1",
    )
    .execute(&app)
    .await
    .expect_err("application role must not inspect revocation ledger");
    assert_eq!(duplicate_key_code(&app_select).as_deref(), Some("42501"));

    let (row_security, forced_row_security): (bool, bool) = sqlx::query_as(
        "SELECT relrowsecurity, relforcerowsecurity
         FROM pg_class WHERE oid = 'human_browser_session_revocations'::regclass",
    )
    .fetch_one(&worker)
    .await
    .expect("read revocation ledger RLS markers");
    assert!(row_security);
    assert!(forced_row_security);

    println!(
        "REAL_BROWSER_SESSION_REVOCATION_SCHEMA=1 migration={max_migration} worker_role=hephaestus_worker app_denied=1 immutable=1 sid_reuse_new_key=1 composite_fk=1"
    );
}

async fn worker_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
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
        .expect("connect worker PostgreSQL role")
}

async fn app_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
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
        .expect("connect application PostgreSQL role")
}

async fn assert_role(pool: &PgPool, expected: &str, superuser: bool, bypass_rls: bool) {
    let (current_user, rolsuper, rolbypassrls): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .expect("read role identity");
    assert_eq!(current_user, expected);
    assert_eq!(rolsuper, superuser);
    assert_eq!(rolbypassrls, bypass_rls);
}

async fn insert_user(pool: &PgPool, user_id: Uuid) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'session revocation user')")
        .bind(user_id)
        .execute(pool)
        .await
        .expect("insert session revocation user");
}

async fn insert_session(pool: &PgPool, user_id: Uuid, sid_digest: &[u8]) -> Uuid {
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
             (id, sid_digest, creation_idempotency_id, creation_request_id,
              identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, gen_random_uuid(), gen_random_uuid(), $3, $4,
                 now(), now() + interval '12 hours')",
    )
    .bind(session_id)
    .bind(sid_digest)
    .bind(vec![0x55_u8; 32])
    .bind(user_id)
    .execute(pool)
    .await
    .expect("insert browser session fixture");
    session_id
}

async fn insert_ledger(
    pool: &PgPool,
    revocation_idempotency_id: Uuid,
    user_id: Uuid,
    sid_digest: &[u8],
    matched_session_id: Option<Uuid>,
    changed: bool,
) {
    sqlx::query(
        "INSERT INTO human_browser_session_revocations
             (revocation_idempotency_id, user_id, sid_digest, request_id,
              matched_session_id, changed)
         VALUES ($1, $2, $3, gen_random_uuid(), $4, $5)",
    )
    .bind(revocation_idempotency_id)
    .bind(user_id)
    .bind(sid_digest)
    .bind(matched_session_id)
    .bind(changed)
    .execute(pool)
    .await
    .expect("insert browser session revocation ledger row");
}

fn duplicate_key_code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(|code| code.to_string())
}

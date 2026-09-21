//! Real-role migration and verifier coverage for human browser sessions.
//!
//! This opt-in test needs a disposable `PostgreSQL` instance with the application
//! and worker roles. The validation runner must provide the test URL and assert
//! the `REAL_BROWSER_SESSION_SCHEMA=1` marker.

use identity_domain::{
    AuthenticatedIdentity, BrowserSessionId, BrowserSessionSid, RequestId, UserId,
    browser_session_identity_binding_digest, browser_session_sid_digest,
};
use serial_test::serial;
use sqlx::{FromRow, PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 85;

#[derive(Debug, PartialEq, Eq, FromRow)]
struct VerifiedSessionRow {
    session_id: Uuid,
    user_id: Uuid,
}

// Keep the real-role matrix together so each result is checked against the
// same migrated disposable database and no lifecycle case can be omitted.
#[allow(clippy::too_many_lines)]
#[tokio::test]
#[serial]
async fn browser_session_schema_enforces_roles_lifecycle_and_verifier() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping browser session schema test: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0085");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker");
    assert!(
        max_migration >= EXPECTED_MIGRATION,
        "browser session migration requires head >= {EXPECTED_MIGRATION}, got {max_migration}"
    );

    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    assert_role(&app, "hephaestus_app", false, false).await;
    assert_role(&worker, "hephaestus_worker", false, true).await;

    let user_id = Uuid::new_v4();
    let inactive_user_id = Uuid::new_v4();
    insert_user(&worker, user_id, "browser session user").await;
    insert_user(&worker, inactive_user_id, "inactive browser session user").await;

    let valid_sid = BrowserSessionSid::new();
    let valid_digest = browser_session_sid_digest(valid_sid).as_bytes().to_vec();
    let valid_creation_idempotency_id = Uuid::new_v4();
    let valid_creation_request_id = Uuid::new_v4();
    let verified_identity = AuthenticatedIdentity::new(
        UserId::from_uuid(user_id),
        "https://issuer.example",
        "browser-subject",
        serde_json::Value::Null,
        RequestId::from_uuid(valid_creation_request_id),
    );
    let valid_identity_binding_digest = browser_session_identity_binding_digest(&verified_identity)
        .as_bytes()
        .to_vec();
    let valid_session_id = insert_session_with_fields(
        &worker,
        user_id,
        valid_digest.clone(),
        valid_creation_idempotency_id,
        valid_creation_request_id,
        valid_identity_binding_digest.clone(),
        SessionTimes::Active,
    )
    .await;

    assert_eq!(
        verify(&app, &valid_digest, user_id).await,
        Some(VerifiedSessionRow {
            session_id: valid_session_id,
            user_id
        }),
    );
    assert!(
        verify(&app, &valid_digest, inactive_user_id)
            .await
            .is_none()
    );
    assert!(verify(&app, &valid_digest, Uuid::new_v4()).await.is_none());
    assert!(verify(&app, &[0_u8; 32], user_id).await.is_none());

    let future_sid = BrowserSessionSid::new();
    let future_digest = browser_session_sid_digest(future_sid).as_bytes().to_vec();
    insert_session(
        &worker,
        user_id,
        future_digest.clone(),
        SessionTimes::Future,
    )
    .await;
    assert!(verify(&app, &future_digest, user_id).await.is_none());

    let expired_sid = BrowserSessionSid::new();
    let expired_digest = browser_session_sid_digest(expired_sid).as_bytes().to_vec();
    insert_session(
        &worker,
        user_id,
        expired_digest.clone(),
        SessionTimes::Expired,
    )
    .await;
    assert!(verify(&app, &expired_digest, user_id).await.is_none());

    let revoked_sid = BrowserSessionSid::new();
    let revoked_digest = browser_session_sid_digest(revoked_sid).as_bytes().to_vec();
    let revoked_id = insert_session(
        &worker,
        user_id,
        revoked_digest.clone(),
        SessionTimes::Active,
    )
    .await;
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
         WHERE id = $1",
    )
    .bind(revoked_id)
    .execute(&worker)
    .await
    .expect("worker may revoke a session");
    assert!(verify(&app, &revoked_digest, user_id).await.is_none());

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(inactive_user_id)
        .execute(&worker)
        .await
        .expect("worker may suspend fixture user");
    let inactive_sid = BrowserSessionSid::new();
    let inactive_digest = browser_session_sid_digest(inactive_sid).as_bytes().to_vec();
    insert_session(
        &worker,
        inactive_user_id,
        inactive_digest.clone(),
        SessionTimes::Active,
    )
    .await;
    assert!(
        verify(&app, &inactive_digest, inactive_user_id)
            .await
            .is_none()
    );

    assert_denied(&app, DeniedOperation::Select).await;
    assert_denied(&app, DeniedOperation::Insert).await;
    assert_denied(&app, DeniedOperation::Update).await;
    assert_denied(&app, DeniedOperation::Delete).await;

    let mut pinned_app = app.acquire().await.expect("pin application connection");
    sqlx::query(
        "CREATE TEMP TABLE human_browser_sessions (
            id uuid, sid_digest bytea, user_id uuid, issued_at timestamptz,
            expires_at timestamptz, revoked_at timestamptz
         )",
    )
    .execute(&mut *pinned_app)
    .await
    .expect("application role may create a temporary shadow table");
    let fake_sid = BrowserSessionSid::new();
    let fake_digest = browser_session_sid_digest(fake_sid).as_bytes().to_vec();
    sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, user_id, issued_at, expires_at, revoked_at)
         VALUES ($1, $2, $3, now(), now() + interval '12 hours', NULL)",
    )
    .bind(Uuid::new_v4())
    .bind(&fake_digest)
    .bind(user_id)
    .execute(&mut *pinned_app)
    .await
    .expect("insert fake shadow row");
    assert_eq!(
        verify_connection(&mut pinned_app, &valid_digest, user_id)
            .await
            .map(|row| row.session_id),
        Some(valid_session_id),
        "qualified verifier must return the genuine public control row"
    );
    assert!(
        verify_connection(&mut pinned_app, &fake_digest, user_id)
            .await
            .is_none(),
        "a temporary shadow row must not bypass the verifier"
    );

    let duplicate = sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')",
    )
    .bind(Uuid::new_v4())
    .bind(valid_digest.clone())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(valid_identity_binding_digest.clone())
    .bind(user_id)
    .execute(&worker)
    .await
    .expect_err("SID digest uniqueness must be enforced");
    assert_eq!(
        duplicate
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23505")
    );
    let duplicate_idempotency = sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')",
    )
    .bind(Uuid::new_v4())
    .bind(
        browser_session_sid_digest(BrowserSessionSid::new())
            .as_bytes()
            .to_vec(),
    )
    .bind(valid_creation_idempotency_id)
    .bind(Uuid::new_v4())
    .bind(valid_identity_binding_digest.clone())
    .bind(user_id)
    .execute(&worker)
    .await
    .expect_err("creation idempotency uniqueness must be enforced");
    assert_eq!(
        duplicate_idempotency
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23505")
    );

    let bad_digest = sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')",
    )
    .bind(Uuid::new_v4())
    .bind(vec![1_u8; 31])
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![2_u8; 32])
    .bind(user_id)
    .execute(&worker)
    .await
    .expect_err("SID digest length must be enforced");
    assert_eq!(
        bad_digest
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23514")
    );

    let zero_lifetime = sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(
        browser_session_sid_digest(BrowserSessionSid::new())
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![3_u8; 32])
    .bind(user_id)
    .execute(&worker)
    .await
    .expect_err("zero lifetime must be rejected");
    assert_eq!(
        zero_lifetime
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23514")
    );

    let overlong = sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '24 hours 1 second')",
    )
    .bind(Uuid::new_v4())
    .bind(
        browser_session_sid_digest(BrowserSessionSid::new())
            .as_bytes()
            .to_vec(),
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![4_u8; 32])
    .bind(user_id)
    .execute(&worker)
    .await
    .expect_err("lifetime upper bound must be enforced");
    assert_eq!(
        overlong
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23514")
    );

    let immutable_id = insert_session(
        &worker,
        user_id,
        browser_session_sid_digest(BrowserSessionSid::new())
            .as_bytes()
            .to_vec(),
        SessionTimes::Active,
    )
    .await;
    let immutable = sqlx::query("UPDATE human_browser_sessions SET expires_at = expires_at + interval '1 hour' WHERE id = $1")
        .bind(immutable_id)
        .execute(&worker)
        .await
        .expect_err("session expiry must be immutable");
    assert_eq!(
        immutable
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    let identity_change =
        sqlx::query("UPDATE human_browser_sessions SET user_id = $2 WHERE id = $1")
            .bind(immutable_id)
            .bind(inactive_user_id)
            .execute(&worker)
            .await
            .expect_err("session user identity must be immutable");
    assert_eq!(
        identity_change
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    let idempotency_change = sqlx::query(
        "UPDATE human_browser_sessions
         SET creation_idempotency_id = $2 WHERE id = $1",
    )
    .bind(immutable_id)
    .bind(Uuid::new_v4())
    .execute(&worker)
    .await
    .expect_err("session creation idempotency must be immutable");
    assert_eq!(
        idempotency_change
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    let request_change =
        sqlx::query("UPDATE human_browser_sessions SET creation_request_id = $2 WHERE id = $1")
            .bind(immutable_id)
            .bind(Uuid::new_v4())
            .execute(&worker)
            .await
            .expect_err("session creation request must be immutable");
    assert_eq!(
        request_change
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    let binding_change = sqlx::query(
        "UPDATE human_browser_sessions
         SET identity_binding_digest = $2 WHERE id = $1",
    )
    .bind(immutable_id)
    .bind(vec![5_u8; 32])
    .execute(&worker)
    .await
    .expect_err("session identity binding must be immutable");
    assert_eq!(
        binding_change
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );

    let reopen = sqlx::query("UPDATE human_browser_sessions SET revoked_at = NULL, revocation_reason = NULL WHERE id = $1")
        .bind(revoked_id)
        .execute(&worker)
        .await
        .expect_err("revoked sessions must not reopen");
    assert_eq!(
        reopen
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );

    let worker_delete = sqlx::query("DELETE FROM human_browser_sessions WHERE id = $1")
        .bind(immutable_id)
        .execute(&worker)
        .await
        .expect_err("worker session grant intentionally excludes delete");
    assert_eq!(
        worker_delete
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );

    println!(
        "REAL_BROWSER_SESSION_SCHEMA=1 migration={max_migration} app_role=hephaestus_app verifier=active_expiry_revocation"
    );
}

async fn role_pool(database_url: &str, role: &str) -> PgPool {
    let application_role = match role {
        "hephaestus_app" => true,
        "hephaestus_worker" => false,
        _ => panic!("unsupported test role"),
    };
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _metadata| {
            Box::pin(async move {
                if application_role {
                    sqlx::query("SET ROLE hephaestus_app")
                        .execute(connection)
                        .await
                        .map(|_| ())
                } else {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await
                        .map(|_| ())
                }
            })
        })
        .connect(database_url)
        .await
        .expect("connect restricted PostgreSQL role")
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

async fn insert_user(pool: &PgPool, user_id: Uuid, display_name: &str) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id)
        .bind(display_name)
        .execute(pool)
        .await
        .expect("insert user fixture");
}

#[derive(Clone, Copy)]
enum SessionTimes {
    Active,
    Future,
    Expired,
}

async fn insert_session(
    pool: &PgPool,
    user_id: Uuid,
    digest: Vec<u8>,
    times: SessionTimes,
) -> Uuid {
    insert_session_with_fields(
        pool,
        user_id,
        digest,
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![0x55_u8; 32],
        times,
    )
    .await
}

async fn insert_session_with_fields(
    pool: &PgPool,
    user_id: Uuid,
    digest: Vec<u8>,
    creation_idempotency_id: Uuid,
    creation_request_id: Uuid,
    identity_binding_digest: Vec<u8>,
    times: SessionTimes,
) -> Uuid {
    let id = BrowserSessionId::new().as_uuid();
    match times {
        SessionTimes::Active => {
            sqlx::query("INSERT INTO human_browser_sessions (id, sid_digest, creation_idempotency_id, creation_request_id, identity_binding_digest, user_id, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')")
                .bind(id).bind(&digest).bind(creation_idempotency_id).bind(creation_request_id).bind(&identity_binding_digest).bind(user_id).execute(pool).await
        }
        SessionTimes::Future => {
            sqlx::query("INSERT INTO human_browser_sessions (id, sid_digest, creation_idempotency_id, creation_request_id, identity_binding_digest, user_id, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, now() + interval '1 second', now() + interval '12 hours')")
                .bind(id).bind(&digest).bind(creation_idempotency_id).bind(creation_request_id).bind(&identity_binding_digest).bind(user_id).execute(pool).await
        }
        SessionTimes::Expired => {
            sqlx::query("INSERT INTO human_browser_sessions (id, sid_digest, creation_idempotency_id, creation_request_id, identity_binding_digest, user_id, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, now() - interval '2 hours', now() - interval '1 second')")
                .bind(id).bind(&digest).bind(creation_idempotency_id).bind(creation_request_id).bind(&identity_binding_digest).bind(user_id).execute(pool).await
        }
    }
    .expect("insert session fixture");
    id
}

async fn verify(pool: &PgPool, digest: &[u8], user_id: Uuid) -> Option<VerifiedSessionRow> {
    sqlx::query_as::<_, VerifiedSessionRow>(
        "SELECT session_id, user_id
         FROM authenticate_human_browser_session($1, $2)",
    )
    .bind(digest.to_vec())
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .expect("call application-role session verifier")
}

async fn verify_connection(
    connection: &mut sqlx::PgConnection,
    digest: &[u8],
    user_id: Uuid,
) -> Option<VerifiedSessionRow> {
    sqlx::query_as::<_, VerifiedSessionRow>(
        "SELECT session_id, user_id
         FROM authenticate_human_browser_session($1, $2)",
    )
    .bind(digest.to_vec())
    .bind(user_id)
    .fetch_optional(connection)
    .await
    .expect("call application-role session verifier on pinned connection")
}

#[derive(Clone, Copy)]
enum DeniedOperation {
    Select,
    Insert,
    Update,
    Delete,
}

async fn assert_denied(pool: &PgPool, operation: DeniedOperation) {
    let result = match operation {
        DeniedOperation::Select => {
            sqlx::query("SELECT id FROM public.human_browser_sessions LIMIT 1")
                .execute(pool)
                .await
        }
        DeniedOperation::Insert => {
            sqlx::query(
                "INSERT INTO public.human_browser_sessions
                 (id, sid_digest, creation_idempotency_id, creation_request_id,
                  identity_binding_digest, user_id, issued_at, expires_at)
                 VALUES (gen_random_uuid(), decode(repeat('00', 32), 'hex'),
                         gen_random_uuid(), gen_random_uuid(),
                         decode(repeat('11', 32), 'hex'), gen_random_uuid(),
                         now(), now() + interval '12 hours')",
            )
            .execute(pool)
            .await
        }
        DeniedOperation::Update => {
            sqlx::query("UPDATE public.human_browser_sessions SET expires_at = expires_at")
                .execute(pool)
                .await
        }
        DeniedOperation::Delete => {
            sqlx::query("DELETE FROM public.human_browser_sessions")
                .execute(pool)
                .await
        }
    };
    let error = result.expect_err("application role must not access session table");
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
}

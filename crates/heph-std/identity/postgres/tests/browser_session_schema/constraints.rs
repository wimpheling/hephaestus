//! Role, shadow-table, constraint, and immutability assertions.

use identity_domain::{BrowserSessionSid, browser_session_sid_digest};
use uuid::Uuid;

use super::Fixture;
use super::helpers::{
    DeniedOperation, SessionTimes, assert_denied, insert_session, verify_connection,
};

pub async fn exercise_roles_and_constraints(fixture: &Fixture, revoked_id: Uuid) {
    assert_denied_and_shadow(fixture).await;
    assert_unique_constraints(fixture).await;
    assert_lifetime_constraints(fixture).await;
    let immutable_id = assert_immutable_fields(fixture).await;
    assert_revocation_and_delete(fixture, revoked_id, immutable_id).await;
}

async fn assert_denied_and_shadow(fixture: &Fixture) {
    assert_denied(&fixture.app, DeniedOperation::Select).await;
    assert_denied(&fixture.app, DeniedOperation::Insert).await;
    assert_denied(&fixture.app, DeniedOperation::Update).await;
    assert_denied(&fixture.app, DeniedOperation::Delete).await;

    let mut pinned_app = fixture
        .app
        .acquire()
        .await
        .expect("pin application connection");
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
    .bind(fixture.user_id)
    .execute(&mut *pinned_app)
    .await
    .expect("insert fake shadow row");
    assert_eq!(
        verify_connection(&mut pinned_app, &fixture.valid_digest, fixture.user_id)
            .await
            .map(|row| row.session_id),
        Some(fixture.valid_session_id),
        "qualified verifier must return the genuine public control row"
    );
    assert!(
        verify_connection(&mut pinned_app, &fake_digest, fixture.user_id)
            .await
            .is_none(),
        "a temporary shadow row must not bypass the verifier"
    );
}

async fn assert_unique_constraints(fixture: &Fixture) {
    let duplicate = sqlx::query(
        "INSERT INTO human_browser_sessions
         (id, sid_digest, creation_idempotency_id, creation_request_id,
          identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.valid_digest.clone())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fixture.valid_identity_binding_digest.clone())
    .bind(fixture.user_id)
    .execute(&fixture.worker)
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
    .bind(fixture.valid_creation_idempotency_id)
    .bind(Uuid::new_v4())
    .bind(fixture.valid_identity_binding_digest.clone())
    .bind(fixture.user_id)
    .execute(&fixture.worker)
    .await
    .expect_err("creation idempotency uniqueness must be enforced");
    assert_eq!(
        duplicate_idempotency
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23505")
    );
}

async fn assert_lifetime_constraints(fixture: &Fixture) {
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
    .bind(fixture.user_id)
    .execute(&fixture.worker)
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
    .bind(fixture.user_id)
    .execute(&fixture.worker)
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
    .bind(fixture.user_id)
    .execute(&fixture.worker)
    .await
    .expect_err("lifetime upper bound must be enforced");
    assert_eq!(
        overlong
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23514")
    );
}

async fn assert_immutable_fields(fixture: &Fixture) -> Uuid {
    let immutable_id = insert_session(
        &fixture.worker,
        fixture.user_id,
        browser_session_sid_digest(BrowserSessionSid::new())
            .as_bytes()
            .to_vec(),
        SessionTimes::Active,
    )
    .await;
    let immutable = sqlx::query("UPDATE human_browser_sessions SET expires_at = expires_at + interval '1 hour' WHERE id = $1")
        .bind(immutable_id)
        .execute(&fixture.worker)
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
            .bind(fixture.inactive_user_id)
            .execute(&fixture.worker)
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
    .execute(&fixture.worker)
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
            .execute(&fixture.worker)
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
    .execute(&fixture.worker)
    .await
    .expect_err("session identity binding must be immutable");
    assert_eq!(
        binding_change
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    immutable_id
}

async fn assert_revocation_and_delete(fixture: &Fixture, revoked_id: Uuid, immutable_id: Uuid) {
    let reopen = sqlx::query("UPDATE human_browser_sessions SET revoked_at = NULL, revocation_reason = NULL WHERE id = $1")
        .bind(revoked_id)
        .execute(&fixture.worker)
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
        .execute(&fixture.worker)
        .await
        .expect_err("worker session grant intentionally excludes delete");
    assert_eq!(
        worker_delete
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
}

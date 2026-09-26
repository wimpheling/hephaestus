//! Shared setup and assertions for browser-session schema coverage.

use identity_domain::{
    AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId,
    browser_session_identity_binding_digest, browser_session_sid_digest,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 85;

#[path = "constraints.rs"]
mod constraints;
#[path = "helpers.rs"]
mod helpers;

use helpers::{
    SessionTimes, VerifiedSessionRow, assert_role, insert_session, insert_session_with_fields,
    insert_user, role_pool, verify,
};

pub use constraints::exercise_roles_and_constraints;

#[derive(Clone)]
pub struct Fixture {
    worker: PgPool,
    app: PgPool,
    user_id: Uuid,
    inactive_user_id: Uuid,
    valid_digest: Vec<u8>,
    valid_creation_idempotency_id: Uuid,
    valid_identity_binding_digest: Vec<u8>,
    valid_session_id: Uuid,
}

pub async fn prepare() -> Option<(Fixture, i64)> {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping browser session schema test: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return None;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../migrations")
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

    Some((
        Fixture {
            worker,
            app,
            user_id,
            inactive_user_id,
            valid_digest,
            valid_creation_idempotency_id,
            valid_identity_binding_digest,
            valid_session_id,
        },
        max_migration,
    ))
}

pub async fn exercise_lifecycle(fixture: &Fixture) -> Uuid {
    assert_eq!(
        verify(&fixture.app, &fixture.valid_digest, fixture.user_id).await,
        Some(VerifiedSessionRow {
            session_id: fixture.valid_session_id,
            user_id: fixture.user_id
        }),
    );
    assert!(
        verify(
            &fixture.app,
            &fixture.valid_digest,
            fixture.inactive_user_id
        )
        .await
        .is_none()
    );
    assert!(
        verify(&fixture.app, &fixture.valid_digest, Uuid::new_v4())
            .await
            .is_none()
    );
    assert!(
        verify(&fixture.app, &[0_u8; 32], fixture.user_id)
            .await
            .is_none()
    );

    let future_sid = BrowserSessionSid::new();
    let future_digest = browser_session_sid_digest(future_sid).as_bytes().to_vec();
    insert_session(
        &fixture.worker,
        fixture.user_id,
        future_digest.clone(),
        SessionTimes::Future,
    )
    .await;
    assert!(
        verify(&fixture.app, &future_digest, fixture.user_id)
            .await
            .is_none()
    );

    let expired_sid = BrowserSessionSid::new();
    let expired_digest = browser_session_sid_digest(expired_sid).as_bytes().to_vec();
    insert_session(
        &fixture.worker,
        fixture.user_id,
        expired_digest.clone(),
        SessionTimes::Expired,
    )
    .await;
    assert!(
        verify(&fixture.app, &expired_digest, fixture.user_id)
            .await
            .is_none()
    );

    let revoked_sid = BrowserSessionSid::new();
    let revoked_digest = browser_session_sid_digest(revoked_sid).as_bytes().to_vec();
    let revoked_session_id = insert_session(
        &fixture.worker,
        fixture.user_id,
        revoked_digest.clone(),
        SessionTimes::Active,
    )
    .await;
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
         WHERE id = $1",
    )
    .bind(revoked_session_id)
    .execute(&fixture.worker)
    .await
    .expect("worker may revoke a session");
    assert!(
        verify(&fixture.app, &revoked_digest, fixture.user_id)
            .await
            .is_none()
    );

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.inactive_user_id)
        .execute(&fixture.worker)
        .await
        .expect("worker may suspend fixture user");
    let inactive_sid = BrowserSessionSid::new();
    let inactive_digest = browser_session_sid_digest(inactive_sid).as_bytes().to_vec();
    insert_session(
        &fixture.worker,
        fixture.inactive_user_id,
        inactive_digest.clone(),
        SessionTimes::Active,
    )
    .await;
    assert!(
        verify(&fixture.app, &inactive_digest, fixture.inactive_user_id)
            .await
            .is_none()
    );

    revoked_session_id
}

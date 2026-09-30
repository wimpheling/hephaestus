//! Browser-session schema fixture primitives.

use identity_domain::BrowserSessionId;
use sqlx::{FromRow, PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, FromRow)]
pub(super) struct VerifiedSessionRow {
    pub(super) session_id: Uuid,
    pub(super) user_id: Uuid,
}

pub(super) async fn role_pool(database_url: &str, role: &str) -> PgPool {
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

pub(super) async fn assert_role(pool: &PgPool, expected: &str, superuser: bool, bypass_rls: bool) {
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

pub(super) async fn insert_user(pool: &PgPool, user_id: Uuid, display_name: &str) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id)
        .bind(display_name)
        .execute(pool)
        .await
        .expect("insert user fixture");
}

#[derive(Clone, Copy)]
pub(super) enum SessionTimes {
    Active,
    Future,
    Expired,
}

pub(super) async fn insert_session(
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

pub(super) async fn insert_session_with_fields(
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

pub(super) async fn verify(
    pool: &PgPool,
    digest: &[u8],
    user_id: Uuid,
) -> Option<VerifiedSessionRow> {
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

pub(super) async fn verify_connection(
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
pub(super) enum DeniedOperation {
    Select,
    Insert,
    Update,
    Delete,
}

pub(super) async fn assert_denied(pool: &PgPool, operation: DeniedOperation) {
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

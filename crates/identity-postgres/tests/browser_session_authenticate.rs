//! Real-role coverage for application-role browser-session authentication.

use identity_application::{
    BrowserSessionAuthenticationError, CreateBrowserSession, VerifiedBrowserIdentity,
};
use identity_domain::{BrowserSessionSid, RequestId, UserId};
use identity_postgres::PostgresBrowserSessionStore;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn application_role_authenticates_created_session_without_table_access() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!(
            "skipping browser session authentication test: HEPHAESTUS_POSTGRES_TEST_URL is unset"
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
        .expect("apply migrations through browser sessions");
    bootstrap.close().await;

    let worker = worker_pool(&database_url).await;
    let application = application_pool(&database_url).await;
    assert_role(&application, "hephaestus_app").await;
    let store = PostgresBrowserSessionStore::new(worker.clone(), application.clone());

    let issuer = format!("https://issuer-{}.example", Uuid::new_v4());
    let subject = format!("subject-{}", Uuid::new_v4());
    let user_id = insert_identity(&worker, &issuer, &subject).await;
    let sid = BrowserSessionSid::new();
    let created = store
        .create_browser_session(CreateBrowserSession {
            request_id: RequestId::new(),
            idempotency_seed: [0x4a; 32],
            verified: VerifiedBrowserIdentity { issuer, subject },
            sid,
        })
        .await
        .expect("create browser session through worker role");

    let direct_select = sqlx::query("SELECT id FROM human_browser_sessions LIMIT 1")
        .fetch_one(&application)
        .await
        .expect_err("application role must not read the session table directly");
    assert_eq!(
        direct_select
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );

    let authenticated = store
        .authenticate_browser_session(user_id, sid)
        .await
        .expect("application-role verifier should authenticate the created session");
    assert_eq!(authenticated, created.metadata);

    assert!(matches!(
        store
            .authenticate_browser_session(user_id, BrowserSessionSid::new())
            .await,
        Err(BrowserSessionAuthenticationError::Unauthenticated)
    ));
    assert!(matches!(
        store.authenticate_browser_session(UserId::new(), sid).await,
        Err(BrowserSessionAuthenticationError::Unauthenticated)
    ));

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(user_id.as_uuid())
        .execute(&worker)
        .await
        .expect("suspend browser-session user");
    assert!(matches!(
        store.authenticate_browser_session(user_id, sid).await,
        Err(BrowserSessionAuthenticationError::Unauthenticated)
    ));

    application.close().await;
    assert!(matches!(
        store.authenticate_browser_session(user_id, sid).await,
        Err(BrowserSessionAuthenticationError::Unavailable)
    ));
    println!(
        "REAL_BROWSER_SESSION_AUTH=1 app_role=hephaestus_app direct_table_select=denied wrong_sid_user=suppressed suspended_user=suppressed closed_pool=unavailable"
    );
}

async fn worker_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
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
        .expect("connect application role")
}

async fn insert_identity(pool: &PgPool, issuer: &str, subject: &str) -> UserId {
    let user_id = Uuid::new_v4();
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

async fn assert_role(pool: &PgPool, expected: &str) {
    let (current_user, rolsuper, rolbypassrls): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .expect("read role identity");
    assert_eq!(current_user, expected);
    assert!(
        !rolsuper,
        "application verifier must not use a superuser role"
    );
    assert!(
        !rolbypassrls,
        "application verifier must not bypass row-level security"
    );
}

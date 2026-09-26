use super::*;

pub async fn assert_rejected_code<F, T>(operation: F, expected_code: &str, reason: &str)
where
    F: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    let result = timeout(Duration::from_secs(5), operation)
        .await
        .expect("schema operation did not finish");
    let error = match result {
        Ok(_) => panic!("schema operation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_database_code(&error, expected_code, reason);
}

pub fn assert_database_code(error: &sqlx::Error, expected_code: &str, reason: &str) {
    let actual_code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code);
    assert_eq!(
        actual_code.as_deref(),
        Some(expected_code),
        "unexpected SQLSTATE for {reason}"
    );
}

pub fn take_query_error<T>(result: Result<T, sqlx::Error>, message: &str) -> sqlx::Error {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

// These helpers match the existing release schema tests. The test URL's login
// role must be allowed to SET ROLE to each restricted disposable-db role.
/// Exercises the first adapter slice against the complete 0089 fixture. The
/// schema matrix above remains the owner of SQLSTATE and lifecycle checks.
pub async fn assert_exchange_denial_unchanged(pool: &PgPool, secret: UiBrowserHandoffSecret) {
    let digest = secret.digest().as_bytes().to_vec();
    let handoff_id: Uuid =
        sqlx::query_scalar("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1")
            .bind(&digest)
            .fetch_one(pool)
            .await
            .expect("find denied exchange handoff");
    let consumed_at: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(handoff_id)
            .fetch_one(pool)
            .await
            .expect("read denied exchange consumption");
    let children: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(handoff_id)
            .fetch_one(pool)
            .await
            .expect("count denied exchange children");
    assert!(
        consumed_at.is_none(),
        "denied exchange consumed its handoff"
    );
    assert_eq!(children, 0, "denied exchange created a child");
}

pub async fn issue_handoff(
    store: &PgUiBrowserSessionStore,
    actor: UserId,
    parent: BrowserSessionId,
    installation: UiInstallationId,
    generation: UiInstallationGenerationId,
    route: UiBrowserRoute,
    secret: UiBrowserHandoffSecret,
) {
    store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route,
            secret,
        })
        .await
        .expect("issue exchange fixture handoff");
}

pub async fn parallel_role_pool(database_url: &str, role: &str, application_name: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            let application_name = application_name.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                let application_name = application_name.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role)
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(application_name)
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
                        .bind(Uuid::nil().to_string())
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect parallel restricted PostgreSQL role")
}

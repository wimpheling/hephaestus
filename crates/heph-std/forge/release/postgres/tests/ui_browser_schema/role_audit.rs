use super::*;

pub async fn role_pool(database_url: &str, role: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role.clone())
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(format!("ui-browser-{role}"))
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
        .expect("connect restricted PostgreSQL role")
}

pub async fn wait_for_named_lock_waiter(admin: &PgPool, application_name: &str, blocker_pid: i32) {
    for _ in 0..200 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE application_name = $1
               AND wait_event_type = 'Lock'
               AND $2 = ANY(pg_blocking_pids(pid))",
        )
        .bind(application_name)
        .bind(blocker_pid)
        .fetch_one(admin)
        .await
        .expect("inspect issuance lock waiter");
        if waiting > 0 {
            println!(
                "REAL_UI_BROWSER_ISSUE_LOCK_BARRIER=1 application_name={application_name} blocker_pid={blocker_pid}"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for issuance lock waiter {application_name}");
}

pub async fn wait_for_named_exchange_lock_waiter(
    admin: &PgPool,
    application_name: &str,
    blocker_pid: i32,
) {
    for _ in 0..1000 {
        let waiting: i64 = sqlx::query_scalar(
            "WITH RECURSIVE blocker_chain(pid, blocking_pid, depth) AS (
                 SELECT activity.pid, unnest(pg_blocking_pids(activity.pid)), 0
                 FROM pg_stat_activity AS activity
                 WHERE activity.application_name = $1
                   AND activity.wait_event_type = 'Lock'
                 UNION ALL
                 SELECT blocker_chain.pid,
                        unnest(pg_blocking_pids(blocker_chain.blocking_pid)),
                        blocker_chain.depth + 1
                 FROM blocker_chain
                 WHERE blocker_chain.depth < 4
             )
             SELECT count(*) FROM blocker_chain WHERE blocking_pid = $2",
        )
        .bind(application_name)
        .bind(blocker_pid)
        .fetch_one(admin)
        .await
        .expect("inspect exchange lock waiter");
        if waiting > 0 {
            println!(
                "REAL_UI_BROWSER_EXCHANGE_LOCK_BARRIER=1 application_name={application_name} blocker_pid={blocker_pid}"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for exchange lock waiter {application_name}");
}

pub async fn assert_role(pool: &PgPool, expected: &str, superuser: bool, bypass_rls: bool) {
    let row: (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .expect("read PostgreSQL role identity");
    assert_eq!(row, (expected.to_owned(), superuser, bypass_rls));
}

pub async fn assert_audit_row(
    pool: &PgPool,
    request_id: RequestId,
    surface: &str,
    decision: &str,
    outcome: &str,
    reason: &str,
) {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT surface, decision, outcome, reason_code
         FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = $2",
    )
    .bind(request_id.as_uuid())
    .bind(surface)
    .fetch_all(pool)
    .await
    .expect("read UI request audit row");
    assert_eq!(rows.len(), 1, "expected one {surface} audit row");
    assert_eq!(
        rows[0],
        (
            surface.into(),
            decision.into(),
            outcome.into(),
            reason.into()
        )
    );
}

// This assertion keeps every verified foreign-key context field explicit so
// an audit row cannot silently omit one relationship.
#[allow(clippy::too_many_arguments)]
pub async fn assert_audit_context(
    pool: &PgPool,
    request_id: RequestId,
    surface: &str,
    actor: Uuid,
    organization: Uuid,
    installation: Uuid,
    generation: Uuid,
    child_session: Option<Uuid>,
) {
    type AuditContext = (
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );
    let context: AuditContext = sqlx::query_as(
        "SELECT actor_id, organization_id, installation_id, generation_id,
                child_session_id, gateway_id, gateway_revision_id
         FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = $2",
    )
    .bind(request_id.as_uuid())
    .bind(surface)
    .fetch_one(pool)
    .await
    .expect("read UI request audit context");
    assert_eq!(
        context,
        (
            Some(actor),
            Some(organization),
            Some(installation),
            Some(generation),
            child_session,
            None,
            None,
        ),
        "verified context for {surface} audit row"
    );
}

pub async fn assert_audit_actor_only(pool: &PgPool, request_id: RequestId, actor: Uuid) {
    type AuditSubjectContext = (
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );
    let context: AuditSubjectContext = sqlx::query_as(
        "SELECT actor_id, organization_id, installation_id, generation_id,
                child_session_id
         FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = 'handoff_issue'",
    )
    .bind(request_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read actor-only UI request audit context");
    assert_eq!(context, (Some(actor), None, None, None, None));
}

pub async fn assert_audit_anonymous(pool: &PgPool, request_id: RequestId, surface: &str) {
    type AuditSubjectContext = (
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );
    let context: AuditSubjectContext = sqlx::query_as(
        "SELECT actor_id, organization_id, installation_id, generation_id,
                child_session_id
         FROM ui_request_audit_events
         WHERE request_id = $1 AND surface = $2",
    )
    .bind(request_id.as_uuid())
    .bind(surface)
    .fetch_one(pool)
    .await
    .expect("read anonymous UI request audit context");
    assert_eq!(context, (None, None, None, None, None));
}

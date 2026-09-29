use super::*;
use crate::issue::IssueContext;

#[allow(clippy::too_many_lines)]
pub async fn run(ctx: &IssueContext) {
    let bootstrap = ctx.bootstrap.clone();
    let worker = ctx.worker.clone();
    let store = &ctx.store;
    let fixture = ctx.fixture;
    let actor = ctx.actor;
    let parent = ctx.parent;
    let installation = ctx.installation;
    let generation = ctx.generation;
    let route = ctx.route.clone();
    let baseline = ctx.baseline;
    let barrier_app = role_pool(&ctx.database_url, "hephaestus_app").await;

    let expiring_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
     (id, sid_digest, creation_idempotency_id, creation_request_id,
      identity_binding_digest, user_id, issued_at, expires_at)
     VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp(),
             statement_timestamp() + interval '1 second')",
    )
    .bind(expiring_parent)
    .bind(digest(222))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(223))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed lock-barrier parent");
    let mut account_lock = bootstrap.begin().await.expect("begin account lock barrier");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *account_lock)
        .await
        .expect("read account lock barrier PID");
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(fixture.actor)
        .fetch_one(&mut *account_lock)
        .await
        .expect("hold actor account lock barrier");
    let barrier_store = PgUiBrowserSessionStore::new(worker.clone(), barrier_app);
    let barrier_route = route.clone();
    let barrier_task = tokio::spawn(async move {
        barrier_store
            .create_ui_browser_handoff(CreateUiBrowserHandoff {
                request_id: RequestId::new(),
                actor_id: actor,
                parent_session_id: BrowserSessionId::from_uuid(expiring_parent),
                installation_id: installation,
                generation_id: generation,
                route: barrier_route,
                secret: UiBrowserHandoffSecret::random(),
            })
            .await
    });
    wait_for_named_lock_waiter(&bootstrap, "ui-browser-hephaestus_worker", blocker_pid).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    account_lock
        .commit()
        .await
        .expect("release account lock barrier");
    assert_eq!(
        barrier_task.await.expect("expiry barrier task"),
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    let after_barrier: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count after expiry barrier");
    assert_eq!(after_barrier, baseline);

    let route_denied_request_id = RequestId::new();
    let route_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: route_denied_request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("arbitrary").expect("safe undeclared route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(route_denied, Err(UiBrowserHandoffError::InvalidRoute));
    assert_audit_row(
        &bootstrap,
        route_denied_request_id,
        "handoff_issue",
        "denied",
        "not_attempted",
        "invalid_route",
    )
    .await;
    assert_audit_actor_only(&bootstrap, route_denied_request_id, fixture.actor).await;
    let after_route: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count after route denial");
    assert_eq!(after_route, baseline);

    sqlx::query("REVOKE INSERT ON public.ui_request_audit_events FROM hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("revoke audit insert for denied-operation test");
    let unavailable_audit_request_id = RequestId::new();
    let denied_with_unavailable_audit = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: unavailable_audit_request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("arbitrary").expect("safe undeclared route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(
        denied_with_unavailable_audit,
        Err(UiBrowserHandoffError::InvalidRoute)
    );
    sqlx::query("GRANT INSERT ON public.ui_request_audit_events TO hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("restore audit insert after denied-operation test");
    let unavailable_audit_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE request_id = $1")
            .bind(unavailable_audit_request_id.as_uuid())
            .fetch_one(&bootstrap)
            .await
            .expect("count unavailable audit rows");
    assert_eq!(unavailable_audit_rows, 0);
}

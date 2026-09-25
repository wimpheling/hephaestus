use super::*;
use crate::exchange::ExchangeContext;

#[allow(clippy::too_many_lines)]
pub async fn run(ctx: &ExchangeContext) {
    let bootstrap = ctx.bootstrap.clone();
    let worker = ctx.worker.clone();
    let store = &ctx.store;
    let fixture = ctx.fixture;
    let actor = ctx.actor;
    let parent = ctx.parent;
    let installation = ctx.installation;
    let generation = ctx.generation;
    let route = ctx.route.clone();

    // An audit persistence failure must roll back both the child insertion and
    // one-time handoff consumption. Restoring the grant must allow that same
    // handoff to be retried successfully.
    let rollback_handoff_secret_bytes = test_secret(fixture.actor, 60);
    let rollback_handoff_digest = UiBrowserHandoffSecret::from_bytes(rollback_handoff_secret_bytes)
        .digest()
        .as_bytes()
        .to_vec();
    issue_handoff(
        store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        UiBrowserHandoffSecret::from_bytes(rollback_handoff_secret_bytes),
    )
    .await;
    let rollback_handoff_id: Uuid =
        sqlx::query_scalar("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1")
            .bind(&rollback_handoff_digest)
            .fetch_one(&worker)
            .await
            .expect("find audit rollback handoff");
    let rollback_children_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("count audit rollback children before failure");
    let rollback_consumed_before: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("read audit rollback handoff before failure");
    assert!(rollback_consumed_before.is_none());

    sqlx::query("REVOKE INSERT ON public.ui_request_audit_events FROM hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("revoke audit insert for exchange rollback");
    let rollback_request_id = RequestId::new();
    let rollback = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: rollback_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(rollback_handoff_secret_bytes),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 61)),
        })
        .await;
    assert_eq!(rollback, Err(UiBrowserHandoffError::Unavailable));
    sqlx::query("GRANT INSERT ON public.ui_request_audit_events TO hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("restore audit insert after exchange rollback");

    let rollback_children_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("count audit rollback children after failure");
    let rollback_consumed_after: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("read audit rollback handoff after failure");
    assert_eq!(rollback_children_after, rollback_children_before);
    assert!(rollback_consumed_after.is_none());
    let rollback_audit_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE request_id = $1")
            .bind(rollback_request_id.as_uuid())
            .fetch_one(&bootstrap)
            .await
            .expect("count failed exchange audit rows");
    assert_eq!(rollback_audit_rows, 0);

    let retry_request_id = RequestId::new();
    let retry = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: retry_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(rollback_handoff_secret_bytes),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 61)),
        })
        .await
        .expect("retry exchange after restoring audit insert");
    let retry_children: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_sessions WHERE handoff_id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("count retried exchange child");
    let retry_consumed: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT consumed_at FROM ui_browser_handoffs WHERE id = $1")
            .bind(rollback_handoff_id)
            .fetch_one(&worker)
            .await
            .expect("read retried exchange handoff");
    assert_eq!(retry_children, 1);
    assert!(retry_consumed.is_some());
    assert_audit_row(
        &bootstrap,
        retry_request_id,
        "handoff_exchange",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    assert_audit_context(
        &bootstrap,
        retry_request_id,
        "handoff_exchange",
        fixture.actor,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        Some(retry.context.session_id.as_uuid()),
    )
    .await;
    println!(
        "REAL_UI_BROWSER_EXCHANGE_AUDIT_ROLLBACK=1 child_rollback={} handoff_unconsumed={} retry_children={} retry_consumed={}",
        rollback_children_after == rollback_children_before,
        rollback_consumed_after.is_none(),
        retry_children,
        retry_consumed.is_some(),
    );
}

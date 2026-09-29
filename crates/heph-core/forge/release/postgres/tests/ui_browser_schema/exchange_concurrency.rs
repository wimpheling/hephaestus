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
    let database_url = ctx.database_url.clone();

    // Two named worker connections contend on one handoff row. An external
    // blocker makes both waits observable before release; after release only
    // one can insert a child and consume the handoff.
    let parallel_worker_a =
        parallel_role_pool(&database_url, "hephaestus_worker", "ui-browser-exchange-a").await;
    let parallel_worker_b =
        parallel_role_pool(&database_url, "hephaestus_worker", "ui-browser-exchange-b").await;
    let parallel_app_a = role_pool(&database_url, "hephaestus_app").await;
    let parallel_app_b = role_pool(&database_url, "hephaestus_app").await;
    let parallel_store_a = PgUiBrowserSessionStore::new(parallel_worker_a, parallel_app_a);
    let parallel_store_b = PgUiBrowserSessionStore::new(parallel_worker_b, parallel_app_b);
    let concurrent_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 46));
    let concurrent_digest = concurrent_secret.digest().as_bytes().to_vec();
    issue_handoff(
        store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        concurrent_secret,
    )
    .await;
    let mut exchange_blocker = bootstrap
        .begin()
        .await
        .expect("begin exchange lock barrier");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *exchange_blocker)
        .await
        .expect("read exchange lock barrier PID");
    sqlx::query("SELECT set_config('application_name', 'ui-browser-exchange-blocker', false)")
        .execute(&mut *exchange_blocker)
        .await
        .expect("name exchange lock barrier");
    println!("REAL_UI_BROWSER_EXCHANGE_BLOCKER=1 blocker_pid={blocker_pid}");
    sqlx::query("SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1 FOR UPDATE")
        .bind(&concurrent_digest)
        .fetch_one(&mut *exchange_blocker)
        .await
        .expect("hold exchange handoff lock barrier");
    let first_task = tokio::spawn(async move {
        parallel_store_a
            .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
                request_id: RequestId::new(),
                handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 46)),
                expected_generation_id: generation,
                child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 47)),
            })
            .await
    });
    let second_task = tokio::spawn(async move {
        parallel_store_b
            .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
                request_id: RequestId::new(),
                handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 46)),
                expected_generation_id: generation,
                child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 48)),
            })
            .await
    });
    wait_for_named_exchange_lock_waiter(&bootstrap, "ui-browser-exchange-a", blocker_pid).await;
    wait_for_named_exchange_lock_waiter(&bootstrap, "ui-browser-exchange-b", blocker_pid).await;
    exchange_blocker
        .commit()
        .await
        .expect("release exchange lock barrier");
    let first = first_task.await.expect("first exchange task");
    let second = second_task.await.expect("second exchange task");
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        usize::from(first == Err(UiBrowserHandoffError::InvalidOrExpired))
            + usize::from(second == Err(UiBrowserHandoffError::InvalidOrExpired)),
        1
    );
    let concurrent_children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_browser_sessions WHERE handoff_id =
     (SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1)",
    )
    .bind(
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 46))
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .fetch_one(&worker)
    .await
    .expect("count concurrent children");
    assert_eq!(concurrent_children, 1);
}

//! Complete early sibling reservations, late NOWAIT rollback and lost caller.
use super::{
    controller::{Control, build},
    observations::{rows, wait_blocker},
    queued_fixture::Queued,
};
use run_orchestrator::{RunCleanupRepository, RunCleanupWakeProgress, RunRepository};
use sqlx::PgPool;
use std::sync::atomic::Ordering;

#[sqlx::test(migrations = false)]
async fn sibling_early_wait_precedes_parents_late_nowait_rolls_back_and_lost_caller_replays_exactly(
    pool: PgPool,
) {
    let fixture = crate::fixture::Fixture::with_sibling(&pool).await;
    super::loader::remaining(&pool).await;
    let closure = super::closure::make(&fixture).await;
    recipe_application::PreparedInstanceClosureRepository::close_for_remove(
        &super::closure::service(&fixture),
        &fixture.birth.actor,
        &closure,
    )
    .await
    .unwrap();
    let h = Queued::from_closed(fixture, closure).await;
    h.deliver().await;
    let sibling:(i64,i32,i32)=sqlx::query_as("SELECT recipe_import107_command_mutex(child.import_command_key),keys.first_key,keys.second_key FROM recipe_prepared_instance_imports child CROSS JOIN LATERAL recipe_import113_instance_mutex(child.instance_id) keys WHERE child.deployment_id=$1 AND resource_name='sibling'")
        .bind(h.fixture.birth.intent.id().as_uuid()).fetch_one(&pool).await.unwrap();
    for command in [true, false] {
        let mut holder = pool.begin().await.unwrap();
        if command {
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(sibling.0)
                .execute(&mut *holder)
                .await
                .unwrap();
        } else {
            sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
                .bind(sibling.1)
                .bind(sibling.2)
                .execute(&mut *holder)
                .await
                .unwrap();
        }
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *holder)
            .await
            .unwrap();
        let repository = h.repository();
        let request = h.request.clone();
        let scope = h.fixture.scope.clone();
        let waiter =
            tokio::spawn(async move { repository.prepare_cleanup_wake(&request, &scope).await });
        wait_blocker(&pool, pid, "advisory").await;
        let mut competitor = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM projects WHERE id=$1 FOR NO KEY UPDATE NOWAIT")
            .bind(h.fixture.birth.fixture.consuming_project)
            .execute(&mut *competitor)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM recipe_deployments WHERE id=$1 FOR UPDATE NOWAIT")
            .bind(h.fixture.birth.intent.id().as_uuid())
            .execute(&mut *competitor)
            .await
            .unwrap();
        sqlx::query("SELECT resource_name FROM recipe_deployment_resources WHERE deployment_id=$1 ORDER BY resource_name FOR UPDATE NOWAIT")
            .bind(h.fixture.birth.intent.id().as_uuid()).execute(&mut *competitor).await.unwrap();
        competitor.rollback().await.unwrap();
        holder.rollback().await.unwrap();
        let prepared = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(h.read().await, prepared);
    }
    let before = rows(&pool).await;
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
        .bind(h.request.run_id().as_uuid())
        .execute(&mut *holder)
        .await
        .unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        h.repository()
            .prepare_cleanup_wake(&h.request, &h.fixture.scope),
    )
    .await
    .unwrap()
    .unwrap_err();
    super::execution::repository_code(&error, "55P03");
    assert_eq!(rows(&pool).await, before);
    holder.rollback().await.unwrap();
    let target = h.target().await;
    let control = Control::new();
    control.pause.store(true, Ordering::SeqCst);
    let core = build(&h, control.clone(), None, None);
    let waking = core.clone();
    let request = h.request.clone();
    let caller = tokio::spawn(async move { waking.wake_cleanup(&request).await });
    control.wait_entered().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(
        h.repository().get(h.request.run_id()).await.unwrap().state,
        run_domain::RunState::Cancelled
    );
    assert_eq!(h.target().await, target);
    assert!(
        h.cleanup()
            .recorded_cleanup_receipt(h.request.run_id())
            .await
            .unwrap()
            .is_none()
    );
    let complete: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_cleanup_completions WHERE run_id=$1")
            .bind(h.request.run_id().as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(complete, 0);
    control.release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let done: bool = sqlx::query_scalar("SELECT state='cleaned_up' FROM runs WHERE id=$1")
                .bind(h.request.run_id().as_uuid())
                .fetch_one(&pool)
                .await
                .unwrap();
            if done {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let terminal = h.read().await;
    assert_eq!(h.target().await, target);
    // A competing wake may briefly observe the detached task's live Start claim.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if core.clone().wake_cleanup(&h.request).await.unwrap()
                == RunCleanupWakeProgress::Complete
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let stable = rows(&pool).await;
    assert_eq!(
        core.wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(h.read().await, terminal);
    assert_eq!(rows(&pool).await, stable);
    assert_eq!(control.count("scoped-entered"), 1);
    assert_eq!(control.count("scoped-confirmed"), 1);
    super::observations::unconsumed(&h).await;
    h.fixture.birth.assert_historical_migrations(&pool).await;
}

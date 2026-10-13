//! Actual closure/consumption serialization, then an actual pending-inbox race.
use super::{
    execution::repository_code,
    observations::{held, rows, wait_blocker},
    queued_fixture::Queued,
};
use recipe_application::PreparedInstanceClosureRepository;
use run_orchestrator::RunRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn permanent_closure_excludes_delayed_consumption_and_pending_cancel_race_rolls_back(
    pool: PgPool,
) {
    let fixture = crate::fixture::Fixture::new(&pool, 0).await;
    super::loader::remaining(&pool).await;
    let closure = super::closure::make(&fixture).await;
    let (first, second): (i32, i32) =
        sqlx::query_as("SELECT first_key,second_key FROM recipe_import113_instance_mutex($1)")
            .bind(fixture.start.instance_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
        .bind(first)
        .bind(second)
        .execute(&mut *holder)
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    let service = super::closure::service(&fixture);
    let actor = fixture.birth.actor.clone();
    let closing = closure.clone();
    let closer = tokio::spawn(async move { service.close_for_remove(&actor, &closing).await });
    wait_blocker(&pool, pid, "advisory").await;
    let closure_pid:i32=sqlx::query_scalar("SELECT a.pid FROM pg_stat_activity a WHERE $1=ANY(pg_blocking_pids(a.pid)) AND a.wait_event='advisory'")
        .bind(pid).fetch_one(&pool).await.unwrap();
    let repository = fixture.repository();
    let command = fixture.start.clone();
    let scope = fixture.scope.clone();
    let vm = fixture.vm.clone();
    let consumer = tokio::spawn(async move {
        repository
            .create_run_with_vm_plan(&command, &scope, &vm)
            .await
    });
    wait_blocker(&pool, closure_pid, "advisory").await;
    let mut competitor = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR NO KEY UPDATE NOWAIT")
        .bind(fixture.birth.fixture.consuming_project)
        .execute(&mut *competitor)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM recipe_deployments WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(fixture.birth.intent.id().as_uuid())
        .execute(&mut *competitor)
        .await
        .unwrap();
    competitor.rollback().await.unwrap();
    holder.rollback().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), closer)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), consumer)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    repository_code(&error, "P1171");
    let h = Queued::from_closed(fixture, closure).await;
    super::observations::unconsumed(&h).await;
    let before = rows(&pool).await;
    let mut shorter = h.fixture.birth.worker.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='100ms'")
        .execute(&mut *shorter)
        .await
        .unwrap();
    super::admission::cancel_lock(&mut shorter, &h)
        .await
        .unwrap();
    let milliseconds: bool = sqlx::query_scalar("SELECT current_setting('lock_timeout')='100ms'")
        .fetch_one(&mut *shorter)
        .await
        .unwrap();
    assert!(
        milliseconds,
        "new facade preserves shorter positive operator timeout"
    );
    shorter.rollback().await.unwrap();
    assert_eq!(rows(&pool).await, before);
    let mut pending = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO command_inbox(command_id,command_type,payload) VALUES($1,'cancel_run',$2)",
    )
    .bind(h.cancel.command_id.as_uuid())
    .bind(serde_json::to_value(&h.cancel).unwrap())
    .execute(&mut *pending)
    .await
    .unwrap();
    let row: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(i) FROM command_inbox i WHERE command_id=$1")
            .bind(h.cancel.command_id.as_uuid())
            .fetch_one(&mut *pending)
            .await
            .unwrap();
    assert!(row["processed_at"].is_null());
    let pending_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *pending)
        .await
        .unwrap();
    let repository = h.repository();
    let cancel = h.cancel.clone();
    let writer = tokio::spawn(async move { repository.request_cancel(&cancel).await });
    wait_blocker(&pool, pending_pid, "transactionid").await;
    // Commit schema-valid transport history only after the new writer has passed
    // prequalification and is waiting in the original ON CONFLICT tail.
    pending.commit().await.unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), writer)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    held(&error);
    let mut expected = before.clone();
    expected["command_inbox"].as_array_mut().unwrap().push(row);
    super::observations::sort(&mut expected);
    assert_eq!(rows(&pool).await, expected);
    held(&h.repository().request_cancel(&h.cancel).await.unwrap_err());
    assert_eq!(
        rows(&pool).await,
        expected,
        "pending input cannot stamp Cancel or mutate Run"
    );
    assert!(
        expected["run_cleanup_targets"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    super::observations::unconsumed(&h).await;
    h.fixture.birth.assert_historical_migrations(&pool).await;
}

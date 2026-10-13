use super::{fixture::Fixture, locking::code};
use run_orchestrator::RunRepository;
use sqlx::{PgPool, Postgres, Transaction};
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL"]
async fn original_prepared_sibling_keys_are_reserved_before_any_parent_row(pool: PgPool) {
    let h = Fixture::with_sibling(&pool).await;
    h.consume().await.unwrap();
    h.repository()
        .transition(
            h.start.run_id,
            run_domain::RunState::LeasingVolume,
            None,
            None,
        )
        .await
        .unwrap();
    let sibling: (i64, i32, i32) = sqlx::query_as(
        "SELECT recipe_import107_command_mutex(child.import_command_key),keys.first_key,keys.second_key FROM recipe_prepared_instance_imports child CROSS JOIN LATERAL recipe_import113_instance_mutex(child.instance_id) keys WHERE child.deployment_id=$1 AND child.resource_name='sibling'",
    ).bind(h.birth.intent.id().as_uuid()).fetch_one(&pool).await.unwrap();
    let child_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recipe_prepared_instance_imports WHERE deployment_id=$1",
    )
    .bind(h.birth.intent.id().as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        child_count, 2,
        "both children came from real protected113 whole preparation"
    );
    for command_lock in [true, false] {
        let before = h.rows().await;
        let mut holder = pool.begin().await.unwrap();
        if command_lock {
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
        let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *holder)
            .await
            .unwrap();
        let mut raw = h.birth.worker.begin().await.unwrap();
        let error = sqlx::query("SELECT lock_qualified_invocation119($1,$2)")
            .bind(h.start.run_id.as_uuid())
            .bind(serde_json::to_vec(&h.config.canonical_json()).unwrap())
            .execute(&mut *raw)
            .await
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
        raw.rollback().await.unwrap();
        let volumes = h.volumes();
        let run = h.start.run_id;
        let waiter = tokio::spawn(async move { volumes.load_run_selections(run).await });
        assert_waiting(&pool, holder_pid).await;
        assert_parents_unlocked(&pool, &h).await;
        holder.rollback().await.unwrap();
        let selected = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(selected.selections().is_empty());
        assert_eq!(h.rows().await, before);
    }
}

async fn assert_waiting(pool: &PgPool, holder: i32) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity WHERE $1=ANY(pg_blocking_pids(activity.pid)) AND activity.wait_event='advisory')")
                .bind(holder).fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
}

async fn assert_parents_unlocked(pool: &PgPool, h: &Fixture) {
    let mut competitor: Transaction<'_, Postgres> = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM projects WHERE id=$1 FOR NO KEY UPDATE NOWAIT")
        .bind(h.birth.fixture.consuming_project)
        .execute(&mut *competitor)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM recipe_deployments WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(h.birth.intent.id().as_uuid())
        .execute(&mut *competitor)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM recipe_deployment_resources WHERE deployment_id=$1 ORDER BY resource_name FOR UPDATE NOWAIT")
        .bind(h.birth.intent.id().as_uuid()).execute(&mut *competitor).await.unwrap();
    competitor.rollback().await.unwrap();
}

use super::{observations::rows, wake_fixture::Wake};
use run_orchestrator::RunRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL gate"]
async fn complete_original_sibling_reservations_wait_before_all_parent_locks(pool: PgPool) {
    let h = Wake::from_fixture(
        crate::fixture::Fixture::with_sibling(&pool).await,
        true,
        true,
    )
    .await;
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
        let mut waiter =
            tokio::spawn(async move { repository.prepare_cleanup_wake(&request, &scope).await });
        let waiting=tokio::time::timeout(std::time::Duration::from_secs(5),async {
            loop {
                let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity a WHERE $1=ANY(pg_blocking_pids(a.pid)) AND a.wait_event='advisory')").bind(pid).fetch_one(&pool).await.unwrap();
                if waiting {break;}
                tokio::task::yield_now().await;
            }
        }).await;
        if waiting.is_err() {
            waiter.abort();
            let _ = waiter.await;
            panic!("owning preparation did not wait on actual sibling advisory key");
        }
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
        sqlx::query("SELECT resource_name FROM recipe_deployment_resources WHERE deployment_id=$1 ORDER BY resource_name FOR UPDATE NOWAIT").bind(h.fixture.birth.intent.id().as_uuid()).execute(&mut *competitor).await.unwrap();
        competitor.rollback().await.unwrap();
        holder.rollback().await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), &mut waiter).await;
        let prepared = if let Ok(result) = result {
            result.unwrap().unwrap()
        } else {
            waiter.abort();
            let _ = waiter.await;
            panic!("owning preparation failed to finish after release")
        };
        assert_eq!(h.read().await, prepared);
    }
    h.fixture.birth.assert_historical_migrations(&pool).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn late_nowait_participant_rolls_back_entire_preparation_then_exact_replay(pool: PgPool) {
    let h = Wake::new(&pool, 2).await;
    let before = rows(&pool).await;
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
        .bind(h.fixture.physical.files[0].0)
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
    let run_orchestrator::RepositoryError::Storage(source) = error else {
        panic!("expected actual lock failure: {error:?}")
    };
    assert_eq!(
        super::observations::code(source.downcast_ref::<sqlx::Error>().unwrap()).as_deref(),
        Some("55P03")
    );
    assert_eq!(rows(&pool).await, before);
    holder.rollback().await.unwrap();
    let prepared = h.prepare().await;
    let after = rows(&pool).await;
    assert_eq!(h.prepare().await, prepared);
    assert_eq!(h.read().await, prepared);
    assert_eq!(rows(&pool).await, after);
    assert!(after["run_cleanup_receipts"].as_array().unwrap().is_empty());
    assert!(
        after["run_cleanup_completions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    h.fixture.physical.assert_unchanged();
}

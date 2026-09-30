use super::*;

#[tokio::test]
#[serial]
// The owner-row lock is the production synchronization point: two real
// worker connections are held behind it, then released to race the same
// actor-bound command and prove one committed result is replayed exactly.
#[allow(clippy::too_many_lines)]
async fn install_static_ui_concurrent_exact_replay_has_one_commit() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-static-install-replay").await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    let release_id = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "matrix-concurrent-replay",
    )
    .await;
    let caller_key =
        UiInstallationCallerKey::parse("matrix-concurrent-replay").expect("caller key");
    let target = UiInstallationTarget::project(fixture.first_project);
    let command = install_command(caller_key.as_str(), target, release_id, "docs");

    let mut owner_lock = admin_pool.begin().await.expect("begin owner lock barrier");
    let owner_backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *owner_lock)
        .await
        .expect("read owner lock backend pid");
    sqlx::query("SELECT id FROM projects WHERE id = $1 FOR NO KEY UPDATE")
        .bind(fixture.first_project.as_uuid())
        .fetch_one(&mut *owner_lock)
        .await
        .expect("hold project owner lock");

    let start = Arc::new(tokio::sync::Barrier::new(3));
    let first_start = Arc::clone(&start);
    let first_pool = worker_pool.clone();
    let first_command = command.clone();
    let first_actor = fixture.actor;
    let first_task = tokio::spawn(async move {
        first_start.wait().await;
        ReleaseService::new(first_pool, Arc::new(PostgresMelangeAuthorizer))
            .install_static_ui(&identity(first_actor), first_command)
            .await
    });
    let second_start = Arc::clone(&start);
    let second_pool = worker_pool.clone();
    let second_command = command;
    let second_actor = fixture.actor;
    let second_task = tokio::spawn(async move {
        second_start.wait().await;
        ReleaseService::new(second_pool, Arc::new(PostgresMelangeAuthorizer))
            .install_static_ui(&identity(second_actor), second_command)
            .await
    });
    start.wait().await;
    wait_for_row_lock_waiters(
        &admin_pool,
        "projects",
        owner_backend_pid,
        "heph-static-install-replay",
        2,
        false,
    )
    .await;
    owner_lock
        .commit()
        .await
        .expect("release owner lock barrier");

    let first = first_task
        .await
        .expect("first concurrent install task")
        .expect("first concurrent install");
    let second = second_task
        .await
        .expect("second concurrent install task")
        .expect("exact concurrent replay");
    assert_eq!(first, second);

    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands
         WHERE installation_id = $1",
    )
    .bind(first.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one concurrent command ledger row");
    assert_eq!(command_count, 1);
    let durable_rows: (i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM ui_installations
              WHERE id = $1 AND project_id = $2 AND repository_id IS NULL),
             (SELECT count(*) FROM ui_installation_generations
              WHERE installation_id = $1)",
    )
    .bind(first.installation_id.as_uuid())
    .bind(fixture.first_project.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one concurrent installation and generation");
    assert_eq!(durable_rows, (1, 1));
    let durable_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1
           AND event.aggregate_type = 'project'
           AND event.aggregate_id = $2
           AND event.event_type = 'project.changed'",
    )
    .bind(first.idempotency_id)
    .bind(fixture.first_project.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one concurrent owner event and product outbox");
    assert_eq!(durable_counts, (1, 1));
    println!(
        "REAL_STATIC_INSTALL_REPLAY=1 blocker_pid={owner_backend_pid} command_rows={command_count} \
         installation_rows={} generation_rows={} event_rows={} outbox_rows={}",
        durable_rows.0, durable_rows.1, durable_counts.0, durable_counts.1
    );
}

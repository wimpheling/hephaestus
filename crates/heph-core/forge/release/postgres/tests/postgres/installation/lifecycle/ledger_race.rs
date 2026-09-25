use super::*;

#[tokio::test]
#[serial]
// Two distinct owner rows let both lifecycle transactions pass owner
// authorization independently while contending on the actor command ledger.
// The loser must retry its rolled-back mutation and report changed input.
#[allow(clippy::too_many_lines)]
async fn ui_installation_lifecycle_cross_owner_ledger_race_conflicts() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-ui-lifecycle-ledger-race").await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    let release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "lifecycle-ledger-race",
    )
    .await;
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let first_install = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "lifecycle-ledger-race-first-install",
                UiInstallationTarget::project(fixture.first_project),
                release,
                "docs",
            ),
        )
        .await
        .expect("first race installation");
    let second_install = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "lifecycle-ledger-race-second-install",
                UiInstallationTarget::project(fixture.second_project),
                release,
                "docs",
            ),
        )
        .await
        .expect("second race installation");

    let mut first_owner_lock = admin_pool.begin().await.expect("begin first owner lock");
    sqlx::query("SELECT id FROM projects WHERE id = $1 FOR UPDATE")
        .bind(fixture.first_project.as_uuid())
        .fetch_one(&mut *first_owner_lock)
        .await
        .expect("hold first project owner lock");
    let mut second_owner_lock = admin_pool.begin().await.expect("begin second owner lock");
    sqlx::query("SELECT id FROM projects WHERE id = $1 FOR UPDATE")
        .bind(fixture.second_project.as_uuid())
        .fetch_one(&mut *second_owner_lock)
        .await
        .expect("hold second project owner lock");

    let start = Arc::new(tokio::sync::Barrier::new(3));
    let first_start = Arc::clone(&start);
    let first_pool = worker_pool.clone();
    let first_id = first_install.installation_id;
    let first_generation = first_install.generation_id;
    let actor = fixture.actor;
    let first_task = tokio::spawn(async move {
        first_start.wait().await;
        ReleaseService::new(first_pool, Arc::new(PostgresMelangeAuthorizer))
            .disable_ui_installation(
                &identity(actor),
                DisableUiInstallation {
                    caller_key: UiInstallationCallerKey::parse("lifecycle-ledger-race")
                        .expect("race caller key"),
                    installation_id: first_id,
                    expected_generation_id: Some(first_generation),
                },
            )
            .await
    });
    let second_start = Arc::clone(&start);
    let second_pool = worker_pool.clone();
    let second_id = second_install.installation_id;
    let second_generation = second_install.generation_id;
    let second_actor = fixture.actor;
    let second_task = tokio::spawn(async move {
        second_start.wait().await;
        ReleaseService::new(second_pool, Arc::new(PostgresMelangeAuthorizer))
            .disable_ui_installation(
                &identity(second_actor),
                DisableUiInstallation {
                    caller_key: UiInstallationCallerKey::parse("lifecycle-ledger-race")
                        .expect("race caller key"),
                    installation_id: second_id,
                    expected_generation_id: Some(second_generation),
                },
            )
            .await
    });
    start.wait().await;
    wait_for_named_lock_waiters(&admin_pool, "heph-ui-lifecycle-ledger-race", 2).await;
    let (first_release, second_release) =
        tokio::join!(first_owner_lock.commit(), second_owner_lock.commit());
    first_release.expect("release first owner lock");
    second_release.expect("release second owner lock");

    let first_result = first_task.await.expect("first lifecycle race task");
    let second_result = second_task.await.expect("second lifecycle race task");
    let winner = match (first_result, second_result) {
        (Err(UiInstallationError::IdempotencyConflict), Ok(result))
        | (Ok(result), Err(UiInstallationError::IdempotencyConflict)) => result,
        (first, second) => {
            panic!("unexpected cross-owner ledger race results: {first:?}, {second:?}")
        }
    };
    let loser_id = if winner.installation_id == first_install.installation_id {
        second_install.installation_id
    } else {
        first_install.installation_id
    };
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands
         WHERE actor_id = $1 AND operation = 'disable'
           AND caller_idempotency_key = 'lifecycle-ledger-race'",
    )
    .bind(fixture.actor.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one winning lifecycle command");
    assert_eq!(command_count, 1);
    let event_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1
           AND event.aggregate_type = 'project'
           AND event.aggregate_id IN ($2, $3)
           AND event.event_type = 'project.changed'",
    )
    .bind(winner.idempotency_id)
    .bind(fixture.first_project.as_uuid())
    .bind(fixture.second_project.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one winning lifecycle event and outbox");
    assert_eq!(event_counts, (1, 1));
    let winner_state: String =
        sqlx::query_scalar("SELECT lifecycle FROM ui_installations WHERE id = $1")
            .bind(winner.installation_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("winning installation state");
    let loser_state: String =
        sqlx::query_scalar("SELECT lifecycle FROM ui_installations WHERE id = $1")
            .bind(loser_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("losing installation state");
    assert_eq!(winner_state, "disabled");
    assert_eq!(loser_state, "enabled");
    println!(
        "REAL_UI_LIFECYCLE_LEDGER_RACE=1 winner={} loser={} command_rows={} event_rows={} outbox_rows={}",
        winner.installation_id, loser_id, command_count, event_counts.0, event_counts.1
    );
}

use super::*;

#[tokio::test]
#[serial]
// This uses the real deferred generation validator and a committed parent
// move; it proves the natural post-insert commit error rolls back every row.
#[allow(clippy::too_many_lines)]
async fn install_static_ui_parent_move_rejects_and_rolls_back_naturally() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-static-install-rollback").await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    sqlx::query(
        "UPDATE organization_members
         SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("promote global installation owner");
    let release_id = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "global",
        "matrix-parent-move-rollback",
    )
    .await;
    let source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(release_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("source project");
    let foreign_project = seed_foreign_project(&admin_pool, fixture.actor).await;
    let foreign_organization: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(foreign_project.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("foreign organization");

    let caller_key =
        UiInstallationCallerKey::parse("matrix-parent-move-rollback").expect("caller key");
    let command_identity = UiInstallationCommandIdentity::new(
        fixture.actor.as_uuid(),
        UiInstallationOperation::Install,
        caller_key.clone(),
    );
    let command_key = command_identity.command_key();
    let occurrence_id =
        actor_idempotency_id(fixture.actor.as_uuid().as_bytes(), command_key.as_bytes()).as_uuid();
    let mut move_tx = admin_pool.begin().await.expect("begin source parent move");
    let move_backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *move_tx)
        .await
        .expect("read source move backend pid");
    sqlx::query("UPDATE projects SET organization_id = $1 WHERE id = $2")
        .bind(foreign_organization)
        .bind(source_project)
        .execute(&mut *move_tx)
        .await
        .expect("hold source project organization move");

    let install_pool = worker_pool.clone();
    let install_actor = fixture.actor;
    let install_target = UiInstallationTarget::organization(fixture.organization);
    let install_task = tokio::spawn(async move {
        ReleaseService::new(install_pool, Arc::new(PostgresMelangeAuthorizer))
            .install_static_ui(
                &identity(install_actor),
                InstallStaticUi {
                    caller_key,
                    target: install_target,
                    release_id,
                    ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                },
            )
            .await
    });
    wait_for_row_lock_waiters(
        &admin_pool,
        "projects",
        move_backend_pid,
        "heph-static-install-rollback",
        1,
        false,
    )
    .await;
    move_tx
        .commit()
        .await
        .expect("commit source project organization move");
    let result = install_task.await.expect("parent move install task");
    assert!(matches!(
        result,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));

    // Restore the unreferenced fixture parent before checking the durable
    // absence, so a failed assertion cannot leave a cross-tenant fixture.
    sqlx::query("UPDATE projects SET organization_id = $1 WHERE id = $2")
        .bind(fixture.organization.as_uuid())
        .bind(source_project)
        .execute(&admin_pool)
        .await
        .expect("restore source project organization");
    let installation_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installations
         WHERE organization_id = $1 AND scope = 'global' AND ui_key = 'docs'",
    )
    .bind(fixture.organization.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("rolled-back installation absence");
    assert_eq!(installation_count, 0);
    let generation_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_generations
         WHERE release_id = $1 AND ui_key = 'docs'",
    )
    .bind(release_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("rolled-back generation absence");
    assert_eq!(generation_count, 0);
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands
         WHERE actor_id = $1 AND caller_idempotency_key = $2",
    )
    .bind(fixture.actor.as_uuid())
    .bind("matrix-parent-move-rollback")
    .fetch_one(&admin_pool)
    .await
    .expect("rolled-back command absence");
    assert_eq!(command_count, 0);
    let durable_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1",
    )
    .bind(occurrence_id)
    .fetch_one(&admin_pool)
    .await
    .expect("rolled-back event and outbox absence");
    assert_eq!(durable_counts, (0, 0));
    println!(
        "REAL_STATIC_INSTALL_ROLLBACK=1 blocker_pid={move_backend_pid} \
         installation_rows={installation_count} generation_rows={generation_count} \
         command_rows={command_count} event_rows={} outbox_rows={}",
        durable_counts.0, durable_counts.1
    );
}

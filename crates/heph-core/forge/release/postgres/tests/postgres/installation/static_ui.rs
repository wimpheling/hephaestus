use super::*;

#[tokio::test]
#[serial]
// This matrix deliberately stays on the first static install boundary: it
// proves organization isolation, owner authorization, replay, and rejection
// before concurrency and lifecycle commands are added.
#[allow(clippy::too_many_lines)]
async fn install_static_ui_project_repository_and_authority_matrix() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool().await else {
        return;
    };
    let worker_role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&worker_pool)
        .await
        .expect("worker role identity");
    assert_eq!(worker_role, "hephaestus_worker");
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));

    let fixture = seed(&admin_pool).await;
    sqlx::query("INSERT INTO repository_managers (repository_id, user_id) VALUES ($1, $2)")
        .bind(fixture.first_repository.as_uuid())
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("seed repository manager");
    let project_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "matrix-project",
    )
    .await;
    let original_identity = identity(fixture.actor);
    let caller_key = UiInstallationCallerKey::parse("matrix-replay").expect("caller key");
    let first = service
        .install_static_ui(
            &original_identity,
            InstallStaticUi {
                caller_key: caller_key.clone(),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id: project_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("project static UI install");
    assert_eq!(first.state, release_domain::UiInstallationState::Enabled);

    let replay = service
        .install_static_ui(
            &identity(fixture.actor),
            InstallStaticUi {
                caller_key: caller_key.clone(),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id: project_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("exact project replay");
    assert_eq!(replay, first);
    let stored_request: Uuid = sqlx::query_scalar(
        "SELECT request_id FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(first.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("stored original request");
    assert_eq!(stored_request, original_identity.request_id.as_uuid());
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
    .expect("project owner event and product outbox");
    assert_eq!(durable_counts, (1, 1));
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(first.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one installation command");
    assert_eq!(command_count, 1);

    let changed_target = service
        .install_static_ui(
            &identity(fixture.actor),
            InstallStaticUi {
                caller_key: caller_key.clone(),
                target: UiInstallationTarget::project(fixture.second_project),
                release_id: project_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        changed_target,
        Err(UiInstallationError::IdempotencyConflict)
    ));
    let changed_release = service
        .install_static_ui(
            &identity(fixture.actor),
            InstallStaticUi {
                caller_key: caller_key.clone(),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id: ReleaseId::new(),
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        changed_release,
        Err(UiInstallationError::IdempotencyConflict)
    ));
    let changed_ui_key = service
        .install_static_ui(
            &identity(fixture.actor),
            InstallStaticUi {
                caller_key,
                target: UiInstallationTarget::project(fixture.first_project),
                release_id: project_release,
                ui_key: release_domain::ui::UiKey::parse("other").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        changed_ui_key,
        Err(UiInstallationError::IdempotencyConflict)
    ));

    let same_org = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-cross-project",
                UiInstallationTarget::project(fixture.second_project),
                project_release,
                "docs",
            ),
        )
        .await
        .expect("same-organization cross-project reuse");
    assert_eq!(same_org.state, release_domain::UiInstallationState::Enabled);

    let repo_fixture = seed(&admin_pool).await;
    sqlx::query("INSERT INTO repository_managers (repository_id, user_id) VALUES ($1, $2)")
        .bind(repo_fixture.first_repository.as_uuid())
        .bind(repo_fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("seed repository-scoped manager");
    let repository_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &repo_fixture,
        "repository",
        "matrix-repository",
    )
    .await;
    let repository_install = service
        .install_static_ui(
            &identity(repo_fixture.actor),
            install_command(
                "matrix-repository-install",
                UiInstallationTarget::repository(repo_fixture.first_repository),
                repository_release,
                "docs",
            ),
        )
        .await
        .expect("repository static UI install");
    assert_eq!(
        repository_install.state,
        release_domain::UiInstallationState::Enabled
    );
    let repository_event: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE occurrence_id = $1 AND aggregate_type = 'repository'
           AND aggregate_id = $2 AND event_type = 'repository.changed'
           AND related_id_one = $3",
    )
    .bind(repository_install.idempotency_id)
    .bind(repo_fixture.first_repository.as_uuid())
    .bind(repo_fixture.first_project.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("repository owner event");
    assert_eq!(repository_event, 1);

    let foreign = seed_foreign_project(&admin_pool, fixture.actor).await;
    let foreign_result = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-cross-organization",
                UiInstallationTarget::project(foreign),
                project_release,
                "docs",
            ),
        )
        .await;
    assert!(matches!(
        foreign_result,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    assert_no_installation(&admin_pool, foreign, "docs").await;

    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.first_project.as_uuid())
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("revoke current project owner");
    let revoked_replay = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-replay",
                UiInstallationTarget::project(fixture.first_project),
                project_release,
                "docs",
            ),
        )
        .await;
    assert!(matches!(
        revoked_replay,
        Err(UiInstallationError::PermissionDenied)
    ));

    let wrong_scope = seed(&admin_pool).await;
    let wrong_scope_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &wrong_scope,
        "repository",
        "matrix-wrong-scope",
    )
    .await;
    let wrong_scope_result = service
        .install_static_ui(
            &identity(wrong_scope.actor),
            install_command(
                "matrix-wrong-scope",
                UiInstallationTarget::project(wrong_scope.first_project),
                wrong_scope_release,
                "docs",
            ),
        )
        .await;
    assert!(matches!(
        wrong_scope_result,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    assert_no_installation(&admin_pool, wrong_scope.first_project, "docs").await;

    let managed = seed(&admin_pool).await;
    let managed_release = publish_managed_release(&admin_pool, &worker_pool, &managed).await;
    let managed_result = service
        .install_static_ui(
            &identity(managed.actor),
            install_command(
                "matrix-managed",
                UiInstallationTarget::project(managed.first_project),
                managed_release,
                "assistant",
            ),
        )
        .await;
    assert!(matches!(
        managed_result,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    assert_no_installation(&admin_pool, managed.first_project, "assistant").await;

    let api = seed(&admin_pool).await;
    let api_release = publish_static_api_release(&admin_pool, &worker_pool, &api).await;
    let api_result = service
        .install_static_ui(
            &identity(api.actor),
            install_command(
                "matrix-static-api",
                UiInstallationTarget::project(api.first_project),
                api_release,
                "docs",
            ),
        )
        .await;
    assert!(matches!(
        api_result,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    assert_no_installation(&admin_pool, api.first_project, "docs").await;
}

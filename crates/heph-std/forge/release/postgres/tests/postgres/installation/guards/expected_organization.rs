use super::*;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn install_ui_expected_organization_is_checked_before_replay() {
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
    let fixture = seed(&admin_pool).await;
    let release_id = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "tenant-organization-guard",
    )
    .await;
    let foreign_project = seed_foreign_project(&admin_pool, fixture.actor).await;
    let foreign_organization: OrganizationId = OrganizationId::from_uuid(
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(foreign_project.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("foreign organization"),
    );
    sqlx::query(
        "UPDATE organization_members
         SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(foreign_organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("promote dual-member actor");

    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let wrong_tenant = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("tenant-wrong").expect("caller key"),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                expected_organization_id: Some(foreign_organization),
                acknowledge_repository_git_access: false,
            },
        )
        .await;
    assert!(matches!(
        wrong_tenant,
        Err(UiInstallationError::OrganizationMismatch)
    ));
    assert_no_installation(&admin_pool, fixture.first_project, "docs").await;

    let matching = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("tenant-match").expect("caller key"),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                expected_organization_id: Some(fixture.organization),
                acknowledge_repository_git_access: false,
            },
        )
        .await
        .expect("matching expected organization");
    assert_eq!(matching.state, release_domain::UiInstallationState::Enabled);

    let same_key = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("tenant-changed").expect("caller key"),
                target: UiInstallationTarget::project(fixture.second_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                expected_organization_id: Some(fixture.organization),
                acknowledge_repository_git_access: false,
            },
        )
        .await
        .expect("same organization command");
    let changed_tenant = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("tenant-changed").expect("caller key"),
                target: UiInstallationTarget::project(fixture.second_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                expected_organization_id: Some(foreign_organization),
                acknowledge_repository_git_access: false,
            },
        )
        .await;
    assert!(matches!(
        changed_tenant,
        Err(UiInstallationError::OrganizationMismatch)
    ));
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(same_key.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("same-key command count");
    assert_eq!(command_count, 1);
    let foreign_receipts: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM ui_installation_commands AS command
         JOIN ui_installations AS installation
           ON installation.id = command.installation_id
         WHERE installation.organization_id = $1
           AND command.caller_idempotency_key = 'tenant-changed'",
    )
    .bind(foreign_organization.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("foreign receipt absence");
    assert_eq!(foreign_receipts, 0);
}

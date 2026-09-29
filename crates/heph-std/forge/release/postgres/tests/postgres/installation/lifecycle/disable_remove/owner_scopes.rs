use super::*;

/// Covers repository and organization owner scopes after project removal.
pub(super) async fn verify_repository_and_global_scopes(
    admin_pool: &PgPool,
    worker_pool: &PgPool,
    service: &ReleaseService,
) {
    verify_repository_scope(admin_pool, worker_pool, service).await;
    verify_global_scope(admin_pool, worker_pool, service).await;
}

async fn verify_repository_scope(
    admin_pool: &PgPool,
    worker_pool: &PgPool,
    service: &ReleaseService,
) {
    let repository_fixture = seed(admin_pool).await;
    sqlx::query("INSERT INTO repository_managers (repository_id, user_id) VALUES ($1, $2)")
        .bind(repository_fixture.first_repository.as_uuid())
        .bind(repository_fixture.actor.as_uuid())
        .execute(admin_pool)
        .await
        .expect("repository manager");
    let repository_release = publish_static_release(
        admin_pool,
        worker_pool,
        &repository_fixture,
        "repository",
        "lifecycle-repository",
    )
    .await;
    let repository_install = service
        .install_static_ui(
            &identity(repository_fixture.actor),
            install_command(
                "lifecycle-repository-install",
                UiInstallationTarget::repository(repository_fixture.first_repository),
                repository_release,
                "docs",
            ),
        )
        .await
        .expect("install repository lifecycle fixture");
    service
        .remove_ui_installation(
            &identity(repository_fixture.actor),
            RemoveUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-repository-remove")
                    .expect("key"),
                installation_id: repository_install.installation_id,
                expected_generation_id: Some(repository_install.generation_id),
            },
        )
        .await
        .expect("remove repository installation");
    let repository_event: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'repository' AND aggregate_id = $1
           AND event_type = 'repository.changed' AND related_id_one = $2
           AND request_id IN (
               SELECT request_id FROM ui_installation_commands
               WHERE installation_id = $3
           )",
    )
    .bind(repository_fixture.first_repository.as_uuid())
    .bind(repository_fixture.first_project.as_uuid())
    .bind(repository_install.installation_id.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("repository owner event");
    assert_eq!(repository_event, 2);
}

async fn verify_global_scope(admin_pool: &PgPool, worker_pool: &PgPool, service: &ReleaseService) {
    let global_fixture = seed(admin_pool).await;
    sqlx::query(
        "UPDATE organization_members SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(global_fixture.organization.as_uuid())
    .bind(global_fixture.actor.as_uuid())
    .execute(admin_pool)
    .await
    .expect("global owner");
    let global_release = publish_static_release(
        admin_pool,
        worker_pool,
        &global_fixture,
        "global",
        "lifecycle-global",
    )
    .await;
    let global_install = service
        .install_static_ui(
            &identity(global_fixture.actor),
            install_command(
                "lifecycle-global-install",
                UiInstallationTarget::organization(global_fixture.organization),
                global_release,
                "docs",
            ),
        )
        .await
        .expect("install global lifecycle fixture");
    service
        .disable_ui_installation(
            &identity(global_fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-global-disable")
                    .expect("key"),
                installation_id: global_install.installation_id,
                expected_generation_id: Some(global_install.generation_id),
            },
        )
        .await
        .expect("disable global installation");
    let global_event: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'organization' AND aggregate_id = $1
           AND event_type = 'organization.changed'
           AND request_id IN (
               SELECT request_id FROM ui_installation_commands
               WHERE installation_id = $2
           )",
    )
    .bind(global_fixture.organization.as_uuid())
    .bind(global_install.installation_id.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("global owner event");
    assert_eq!(global_event, 2);
}

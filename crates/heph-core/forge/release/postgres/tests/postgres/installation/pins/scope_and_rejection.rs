use super::*;

/// Covers organization scoped installation and rejected route publication.
pub(super) async fn verify(admin_pool: &PgPool, worker_pool: &PgPool, service: &ReleaseService) {
    verify_global_install(admin_pool, worker_pool, service).await;
    verify_invalid_route(admin_pool, worker_pool, service).await;
}

async fn verify_global_install(
    admin_pool: &PgPool,
    worker_pool: &PgPool,
    service: &ReleaseService,
) {
    let global_fixture = seed(admin_pool).await;
    sqlx::query(
        "UPDATE organization_members SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(global_fixture.organization.as_uuid())
    .bind(global_fixture.actor.as_uuid())
    .execute(admin_pool)
    .await
    .expect("promote organization owner");
    let global_release =
        publish_managed_release_for_scope(admin_pool, worker_pool, &global_fixture, "global").await;
    let (global_agent_id, global_agent_key): (Uuid, String) =
        sqlx::query_as("SELECT id, agent_key FROM release_agents WHERE release_id = $1")
            .bind(global_release.as_uuid())
            .fetch_one(admin_pool)
            .await
            .expect("global release agent");
    seed_active_ui_gateway(
        admin_pool,
        &global_fixture,
        global_release,
        global_agent_id,
        &global_agent_key,
        "http.service.v1",
        "heph_authenticated",
        "/service",
        &["GET"],
    )
    .await;
    service
        .install_ui(
            &identity(global_fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("general-global").expect("caller key"),
                target: UiInstallationTarget::organization(global_fixture.organization),
                release_id: global_release,
                ui_key: release_domain::ui::UiKey::parse("assistant").expect("UI key"),
                expected_organization_id: None,
                acknowledge_repository_git_access: false,
            },
        )
        .await
        .expect("same-organization global managed/API installation");
}

async fn verify_invalid_route(admin_pool: &PgPool, worker_pool: &PgPool, service: &ReleaseService) {
    let invalid_fixture = seed(admin_pool).await;
    let invalid_release = publish_managed_release(admin_pool, worker_pool, &invalid_fixture).await;
    let (invalid_agent_id, invalid_agent_key): (Uuid, String) =
        sqlx::query_as("SELECT id, agent_key FROM release_agents WHERE release_id = $1")
            .bind(invalid_release.as_uuid())
            .fetch_one(admin_pool)
            .await
            .expect("invalid release agent");
    seed_active_ui_gateway(
        admin_pool,
        &invalid_fixture,
        invalid_release,
        invalid_agent_id,
        &invalid_agent_key,
        "http.service.v1",
        "heph_authenticated",
        "/serviceable",
        &["GET"],
    )
    .await;
    let invalid_event_baseline: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.aggregate_id = $1",
    )
    .bind(invalid_fixture.first_project.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("invalid fixture event baseline");
    let rejected = service
        .install_ui(
            &identity(invalid_fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("general-sibling-route")
                    .expect("caller key"),
                target: UiInstallationTarget::project(invalid_fixture.first_project),
                release_id: invalid_release,
                ui_key: release_domain::ui::UiKey::parse("assistant").expect("UI key"),
                expected_organization_id: None,
                acknowledge_repository_git_access: false,
            },
        )
        .await;
    assert!(matches!(
        rejected,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    let absence: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM ui_installations WHERE project_id = $1),
             (SELECT count(*) FROM ui_installation_generations AS generation
               JOIN ui_installations AS installation
                 ON installation.id = generation.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM ui_installation_bindings AS binding
               JOIN ui_installations AS installation
                 ON installation.id = binding.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM ui_installation_commands AS command
               JOIN ui_installations AS installation
                 ON installation.id = command.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM application_events
              WHERE aggregate_id = $1),
             (SELECT count(outbox.event_id)
              FROM application_events AS event
              LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
              WHERE event.aggregate_id = $1)",
    )
    .bind(invalid_fixture.first_project.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("invalid installation absence");
    assert_eq!(
        absence,
        (
            0,
            0,
            0,
            0,
            invalid_event_baseline.0,
            invalid_event_baseline.1
        )
    );
}

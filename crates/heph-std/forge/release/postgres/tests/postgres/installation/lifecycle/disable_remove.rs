use super::*;

#[path = "disable_remove/owner_scopes.rs"]
mod owner_scopes;

#[tokio::test]
#[serial]
// Disable/remove use only current target management. The matrix deliberately
// revokes source-release use before replaying an exact disable command.
// This exception keeps all owner shapes and post-commit assertions in one
// matrix so replay and terminal-state evidence share one installation.
#[allow(clippy::too_many_lines)]
async fn ui_installation_disable_remove_lifecycle_matrix() {
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
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));

    let fixture = seed(&admin_pool).await;
    let release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "lifecycle-project",
    )
    .await;
    let installation = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "lifecycle-install",
                UiInstallationTarget::project(fixture.first_project),
                release,
                "docs",
            ),
        )
        .await
        .expect("install lifecycle project fixture");
    let disabled = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("disable project installation");
    assert_eq!(
        disabled.state,
        release_domain::UiInstallationState::Disabled
    );
    assert_eq!(disabled.generation_id, installation.generation_id);
    let stored_disable_request: Uuid = sqlx::query_scalar(
        "SELECT request_id FROM ui_installation_commands
         WHERE installation_id = $1 AND operation = 'disable'
           AND caller_idempotency_key = 'lifecycle-disable'",
    )
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("stored disable request provenance");

    let replay = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("exact disable replay");
    assert_eq!(replay, disabled);
    let replayed_disable_request: Uuid = sqlx::query_scalar(
        "SELECT request_id FROM ui_installation_commands
         WHERE installation_id = $1 AND operation = 'disable'
           AND caller_idempotency_key = 'lifecycle-disable'",
    )
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("replayed disable request provenance");
    assert_eq!(replayed_disable_request, stored_disable_request);
    let conflict = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: None,
            },
        )
        .await;
    assert!(matches!(
        conflict,
        Err(UiInstallationError::IdempotencyConflict)
    ));
    let stale = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-stale").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(UiInstallationGenerationId::new()),
            },
        )
        .await;
    assert!(matches!(
        stale,
        Err(UiInstallationError::GenerationConflict)
    ));

    let source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(release.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("source project");
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(source_project)
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("revoke source release use");
    let revoked_replay = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("replay without source permission");
    assert_eq!(revoked_replay, disabled);

    let same_state = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable-again").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("fresh same-state disable");
    assert_eq!(same_state.generation_id, installation.generation_id);
    let removed = service
        .remove_ui_installation(
            &identity(fixture.actor),
            RemoveUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-remove").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("remove project installation");
    assert_eq!(removed.state, release_domain::UiInstallationState::Removed);
    assert_eq!(removed.generation_id, installation.generation_id);
    let remove_replay = service
        .remove_ui_installation(
            &identity(fixture.actor),
            RemoveUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-remove").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("exact remove replay");
    assert_eq!(remove_replay, removed);
    let terminal = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-terminal").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await;
    assert!(matches!(
        terminal,
        Err(UiInstallationError::InvalidTransition)
    ));
    let project_event_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.aggregate_type = 'project' AND event.aggregate_id = $1
           AND event.event_type = 'project.changed'
           AND event.request_id IN (
               SELECT request_id FROM ui_installation_commands
               WHERE installation_id = $2
           )",
    )
    .bind(fixture.first_project.as_uuid())
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("project lifecycle event counts");
    assert_eq!(project_event_counts, (4, 4));
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("project lifecycle command count");
    assert_eq!(command_count, 4);

    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(source_project)
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("restore source release use");
    let reused = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "lifecycle-reused-key",
                UiInstallationTarget::project(fixture.first_project),
                release,
                "docs",
            ),
        )
        .await
        .expect("reuse removed installation key");
    assert_ne!(reused.installation_id, installation.installation_id);

    owner_scopes::verify_repository_and_global_scopes(&admin_pool, &worker_pool, &service).await;
}

use super::*;

// The fixture keeps each immutable gateway revision field explicit so every
// denial case visibly changes only its intended production property.
#[tokio::test]
#[serial]
// Keep the global authorization, tenancy, replay, and event cases in one
// real-PostgreSQL matrix so each assertion shares the same committed fixture.
#[allow(clippy::too_many_lines)]
async fn install_static_ui_global_organization_matrix() {
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
    let global_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "global",
        "matrix-global",
    )
    .await;

    let source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(global_release.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("global release source project");
    let source_organization: Uuid = sqlx::query_scalar(
        "SELECT project.organization_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         JOIN projects AS project ON project.id = repository.project_id
         WHERE release.id = $1",
    )
    .bind(global_release.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("global release source organization");
    assert_eq!(source_organization, fixture.organization.as_uuid());
    assert_ne!(source_project, fixture.first_project.as_uuid());

    let caller_key = UiInstallationCallerKey::parse("matrix-global-replay").expect("caller key");
    let first = service
        .install_static_ui(
            &identity(fixture.actor),
            InstallStaticUi {
                caller_key: caller_key.clone(),
                target: UiInstallationTarget::organization(fixture.organization),
                release_id: global_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("organization static UI install");
    assert_eq!(first.state, release_domain::UiInstallationState::Enabled);
    let owner_shape: (Uuid, Option<Uuid>, Option<Uuid>, String) = sqlx::query_as(
        "SELECT organization_id, project_id, repository_id, scope
         FROM ui_installations WHERE id = $1",
    )
    .bind(first.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("global installation owner shape");
    assert_eq!(
        owner_shape,
        (
            fixture.organization.as_uuid(),
            None,
            None,
            String::from("global")
        )
    );

    let replay = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-replay",
                UiInstallationTarget::organization(fixture.organization),
                global_release,
                "docs",
            ),
        )
        .await
        .expect("exact organization replay");
    assert_eq!(replay, first);
    let durable_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1
           AND event.aggregate_type = 'organization'
           AND event.aggregate_id = $2
           AND event.event_type = 'organization.changed'",
    )
    .bind(first.idempotency_id)
    .bind(fixture.organization.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("organization owner event and product outbox");
    assert_eq!(durable_counts, (1, 1));

    let conflict = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-replay",
                UiInstallationTarget::organization(fixture.organization),
                global_release,
                "other",
            ),
        )
        .await;
    assert!(matches!(
        conflict,
        Err(UiInstallationError::IdempotencyConflict)
    ));

    let foreign_project = seed_foreign_project(&admin_pool, fixture.actor).await;
    let foreign_organization: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(foreign_project.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("foreign organization");
    sqlx::query(
        "UPDATE organization_members
         SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(foreign_organization)
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("promote dual organization owner");
    let foreign_result = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-cross-org",
                UiInstallationTarget::organization(OrganizationId::from_uuid(foreign_organization)),
                global_release,
                "docs",
            ),
        )
        .await;
    assert!(matches!(
        foreign_result,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    let foreign_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installations
         WHERE organization_id = $1 AND scope = 'global' AND ui_key = 'docs'",
    )
    .bind(foreign_organization)
    .fetch_one(&admin_pool)
    .await
    .expect("foreign global installation absence");
    assert_eq!(foreign_count, 0);

    let second = seed(&admin_pool).await;
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(second.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("seed second organization owner");
    let second_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &second,
        "global",
        "matrix-global-second",
    )
    .await;
    let second_source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(second_release.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("second global release source project");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(second_source_project)
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("seed second release use permission");
    let second_install = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-second-org",
                UiInstallationTarget::organization(second.organization),
                second_release,
                "docs",
            ),
        )
        .await
        .expect("same UI key in second organization");
    assert_eq!(
        second_install.state,
        release_domain::UiInstallationState::Enabled
    );
    let same_key_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installations
         WHERE organization_id IN ($1, $2)
           AND scope = 'global' AND ui_key = 'docs' AND lifecycle <> 'removed'",
    )
    .bind(fixture.organization.as_uuid())
    .bind(second.organization.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("same global key in two organizations");
    assert_eq!(same_key_count, 2);

    sqlx::query(
        "UPDATE organization_members
         SET role = 'member'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("revoke current global owner");
    let revoked_replay = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-replay",
                UiInstallationTarget::organization(fixture.organization),
                global_release,
                "docs",
            ),
        )
        .await;
    assert!(matches!(
        revoked_replay,
        Err(UiInstallationError::PermissionDenied)
    ));
}

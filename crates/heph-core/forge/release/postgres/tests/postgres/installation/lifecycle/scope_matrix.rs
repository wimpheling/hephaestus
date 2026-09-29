use super::*;

/// Builds the initial project generation and verifies repository/global scopes.
pub(super) async fn prepare(admin_pool: PgPool, worker_pool: PgPool) -> LifecycleContext {
    let fixture = seed(&admin_pool).await;
    let release_v1 = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "generation-v1",
    )
    .await;
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let installation = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "generation-install",
                UiInstallationTarget::project(fixture.first_project),
                release_v1,
                "docs",
            ),
        )
        .await
        .expect("install first generation");
    let generation_one_no: i64 =
        sqlx::query_scalar("SELECT generation_no FROM ui_installation_generations WHERE id = $1")
            .bind(installation.generation_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("first generation number");
    assert_eq!(generation_one_no, 1);

    let activation = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-activate").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
                release_id: release_v1,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("activate fresh generation");
    assert_ne!(activation.generation_id, installation.generation_id);
    assert_eq!(
        activation.state,
        release_domain::UiInstallationState::Enabled
    );
    let retained_generations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_generations WHERE installation_id = $1",
    )
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("retained generations after activation");
    assert_eq!(retained_generations, 2);
    let current_generation: Uuid =
        sqlx::query_scalar("SELECT current_generation_id FROM ui_installations WHERE id = $1")
            .bind(installation.installation_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("current generation pointer");
    assert_eq!(current_generation, activation.generation_id.as_uuid());
    let context = LifecycleContext {
        admin_pool,
        worker_pool,
        service,
        fixture,
        release_v1,
        installation,
        activation,
    };
    repository_scope(&context).await;
    global_scope(&context).await;
    context
}

async fn repository_scope(context: &LifecycleContext) {
    let admin_pool = &context.admin_pool;
    let worker_pool = &context.worker_pool;
    let service = &context.service;
    let fixture = &context.fixture;
    sqlx::query(
        "INSERT INTO repository_managers (repository_id, user_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(fixture.first_repository.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(admin_pool)
    .await
    .expect("seed repository owner for generation scope");
    let repository_release = publish_static_release(
        admin_pool,
        worker_pool,
        fixture,
        "repository",
        "generation-repository",
    )
    .await;
    let repository_installation = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "generation-repository-install",
                UiInstallationTarget::repository(fixture.first_repository),
                repository_release,
                "docs",
            ),
        )
        .await
        .expect("install repository generation scope");
    let repository_activation = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-repository-activate")
                    .expect("repository activation key"),
                installation_id: repository_installation.installation_id,
                expected_generation_id: Some(repository_installation.generation_id),
                release_id: repository_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("activate repository generation scope");
    assert_ne!(
        repository_activation.generation_id,
        repository_installation.generation_id
    );
    let repository_rollback = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-repository-rollback")
                    .expect("repository rollback key"),
                installation_id: repository_installation.installation_id,
                expected_generation_id: Some(repository_activation.generation_id),
                release_id: repository_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("rollback repository generation scope");
    assert_ne!(
        repository_rollback.generation_id,
        repository_activation.generation_id
    );
}

async fn global_scope(context: &LifecycleContext) {
    let admin_pool = &context.admin_pool;
    let worker_pool = &context.worker_pool;
    let service = &context.service;
    let fixture = &context.fixture;
    sqlx::query(
        "UPDATE organization_members SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(admin_pool)
    .await
    .expect("promote global owner for generation scope");
    let global_release = publish_static_release(
        admin_pool,
        worker_pool,
        fixture,
        "global",
        "generation-global",
    )
    .await;
    let global_installation = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "generation-global-install",
                UiInstallationTarget::organization(fixture.organization),
                global_release,
                "docs",
            ),
        )
        .await
        .expect("install global generation scope");
    let global_activation = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-global-activate")
                    .expect("global activation key"),
                installation_id: global_installation.installation_id,
                expected_generation_id: Some(global_installation.generation_id),
                release_id: global_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("activate global generation scope");
    assert_ne!(
        global_activation.generation_id,
        global_installation.generation_id
    );
    let global_rollback = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-global-rollback")
                    .expect("global rollback key"),
                installation_id: global_installation.installation_id,
                expected_generation_id: Some(global_activation.generation_id),
                release_id: global_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("rollback global generation scope");
    assert_ne!(
        global_rollback.generation_id,
        global_activation.generation_id
    );
}

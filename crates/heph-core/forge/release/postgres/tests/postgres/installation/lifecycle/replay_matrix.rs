use super::*;

/// Verifies replay, CAS, source revocation, rollback, and invalid binding atomicity.
pub(super) async fn verify(context: LifecycleContext) {
    let context = context;
    let release_v2 = verify_replay_and_input(&context).await;
    verify_source_revocation(&context).await;
    let rollback_generation_id = verify_new_caller_and_rollback(&context, release_v2).await;
    verify_invalid_binding(&context, rollback_generation_id).await;
}

async fn verify_replay_and_input(context: &LifecycleContext) -> ReleaseId {
    let admin_pool = &context.admin_pool;
    let worker_pool = &context.worker_pool;
    let service = &context.service;
    let fixture = &context.fixture;
    let release_v1 = context.release_v1;
    let installation = context.installation;
    let activation = context.activation;
    let activation_replay = service
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
        .expect("exact activation replay");
    assert_eq!(activation_replay, activation);
    let release_v2 =
        publish_static_release(admin_pool, worker_pool, fixture, "project", "generation-v2").await;
    let changed_input = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-activate").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
                release_id: release_v2,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        changed_input,
        Err(UiInstallationError::IdempotencyConflict)
    ));
    let stale_cas = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-stale-cas").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
                release_id: release_v1,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        stale_cas,
        Err(UiInstallationError::GenerationConflict)
    ));
    release_v2
}

async fn verify_source_revocation(context: &LifecycleContext) {
    let admin_pool = &context.admin_pool;
    let service = &context.service;
    let fixture = &context.fixture;
    let release_v1 = context.release_v1;
    let installation = context.installation;
    let activation = context.activation;
    let source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(release_v1.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("load source project for release revocation");
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(source_project)
        .bind(fixture.actor.as_uuid())
        .execute(admin_pool)
        .await
        .expect("revoke source release use");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization.as_uuid())
        .bind(fixture.actor.as_uuid())
        .execute(admin_pool)
        .await
        .expect("revoke source organization use");
    let revoked_replay = service
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
        .expect("activation replay after source revocation");
    assert_eq!(revoked_replay, activation);
    sqlx::query(
    "INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
)
.bind(source_project)
.bind(fixture.actor.as_uuid())
.execute(admin_pool)
.await
.expect("restore source release use");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member') ON CONFLICT DO NOTHING",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(admin_pool)
    .await
    .expect("restore source organization use");
}

async fn verify_new_caller_and_rollback(
    context: &LifecycleContext,
    release_v2: ReleaseId,
) -> UiInstallationGenerationId {
    let admin_pool = &context.admin_pool;
    let service = &context.service;
    let fixture = &context.fixture;
    let release_v1 = context.release_v1;
    let installation = context.installation;
    let activation = context.activation;
    let same_input_new_caller = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-activate-again")
                    .expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(activation.generation_id),
                release_id: release_v1,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("fresh same-input activation");
    assert_ne!(
        same_input_new_caller.generation_id,
        activation.generation_id
    );

    let rollback = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-rollback").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(same_input_new_caller.generation_id),
                release_id: release_v2,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("rollback to second release");
    assert_ne!(rollback.generation_id, same_input_new_caller.generation_id);
    assert_eq!(rollback.state, release_domain::UiInstallationState::Enabled);
    let generation_rows: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE id = $2)
         FROM ui_installation_generations WHERE installation_id = $1",
    )
    .bind(installation.installation_id.as_uuid())
    .bind(installation.generation_id.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("all immutable generations retained");
    assert_eq!(generation_rows, (4, 1));
    rollback.generation_id
}

async fn verify_invalid_binding(
    context: &LifecycleContext,
    rollback_generation_id: UiInstallationGenerationId,
) {
    let admin_pool = &context.admin_pool;
    let worker_pool = &context.worker_pool;
    let service = &context.service;
    let fixture = &context.fixture;
    let installation = context.installation;
    let invalid_release = publish_static_api_release(admin_pool, worker_pool, fixture).await;
    let invalid_caller = UiInstallationCallerKey::parse("generation-invalid-binding").expect("key");
    let invalid_operation = UiInstallationCommandIdentity::new(
        fixture.actor.as_uuid(),
        UiInstallationOperation::Rollback,
        invalid_caller.clone(),
    );
    let invalid_occurrence = actor_idempotency_id(
        fixture.actor.as_uuid().as_bytes(),
        invalid_operation.command_key().as_bytes(),
    )
    .as_uuid();
    let before_invalid: (i64, i64, i64, i64) = sqlx::query_as(
    "SELECT\n             (SELECT count(*) FROM ui_installation_generations WHERE installation_id = $1),\n             (SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1),\n             (SELECT count(*) FROM application_events WHERE occurrence_id = $2),\n             (SELECT count(outbox.event_id) FROM application_events AS event\n              LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id\n              WHERE event.occurrence_id = $2)",
)
.bind(installation.installation_id.as_uuid())
.bind(invalid_occurrence)
.fetch_one(admin_pool)
.await
.expect("invalid rollback baseline");
    let invalid = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: invalid_caller,
                installation_id: installation.installation_id,
                expected_generation_id: Some(rollback_generation_id),
                release_id: invalid_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        invalid,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    let after_invalid: (i64, i64, i64, i64) = sqlx::query_as(
    "SELECT\n             (SELECT count(*) FROM ui_installation_generations WHERE installation_id = $1),\n             (SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1),\n             (SELECT count(*) FROM application_events WHERE occurrence_id = $2),\n             (SELECT count(outbox.event_id) FROM application_events AS event\n              LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id\n              WHERE event.occurrence_id = $2)",
)
.bind(installation.installation_id.as_uuid())
.bind(invalid_occurrence)
.fetch_one(admin_pool)
.await
.expect("invalid rollback absence");
    assert_eq!(after_invalid, before_invalid);
    println!(
        "REAL_UI_INSTALLATION_GENERATION_MATRIX=1 generations=4 owner_scopes=project_repository_global replay=1 source_revocation=1 same_input_new_caller=1 stale_cas=1 invalid_binding_no_rows=1"
    );
}

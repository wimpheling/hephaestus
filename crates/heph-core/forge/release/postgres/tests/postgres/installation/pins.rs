use super::*;

#[path = "pins/scope_and_rejection.rs"]
mod scope_and_rejection;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn install_ui_pins_active_gateway_revision_and_exact_published_routes() {
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
    let release_id = publish_managed_release(&admin_pool, &worker_pool, &fixture).await;
    let (release_agent_id, release_agent_key): (Uuid, String) =
        sqlx::query_as("SELECT id, agent_key FROM release_agents WHERE release_id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("published release agent");
    let (gateway_id, revision_id) = seed_active_ui_gateway(
        &admin_pool,
        &fixture,
        release_id,
        release_agent_id,
        &release_agent_key,
        "http.service.v1",
        "heph_authenticated",
        "/service",
        &["GET"],
    )
    .await;

    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let result = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("general-managed-api")
                    .expect("caller key"),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("assistant").expect("UI key"),
                expected_organization_id: None,
                acknowledge_repository_git_access: false,
            },
        )
        .await
        .expect("managed/API installation");
    let rows: Vec<UiBindingRow> = sqlx::query_as(
        "SELECT binding_kind, binding_key, gateway_id, gateway_revision_id,
                release_agent_id, method, route, exposure
         FROM ui_installation_bindings
         WHERE installation_id = $1 AND generation_id = $2
         ORDER BY binding_kind, binding_key",
    )
    .bind(result.installation_id.as_uuid())
    .bind(result.generation_id.as_uuid())
    .fetch_all(&admin_pool)
    .await
    .expect("installation bindings");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| {
        row.2 == gateway_id
            && row.3 == revision_id
            && row.4 == release_agent_id
            && row.7 == "heph_authenticated"
    }));
    assert_eq!(rows[0].5, "GET");
    assert_eq!(rows[0].6, "/service/api");
    assert_eq!(rows[1].5, "GET");
    assert_eq!(rows[1].6, "/service/ui");

    let before_replay_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1",
    )
    .bind(result.idempotency_id)
    .fetch_one(&admin_pool)
    .await
    .expect("original installation event and outbox");
    let before_replay_bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_bindings WHERE installation_id = $1",
    )
    .bind(result.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("original installation binding count");
    sqlx::query(
        "UPDATE gateways
         SET lifecycle = 'paused', active_revision_id = NULL
         WHERE id = $1",
    )
    .bind(gateway_id)
    .execute(&admin_pool)
    .await
    .expect("revoke gateway serving authority");
    let replay = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("general-managed-api")
                    .expect("caller key"),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("assistant").expect("UI key"),
                expected_organization_id: None,
                acknowledge_repository_git_access: false,
            },
        )
        .await
        .expect("exact replay after gateway revocation");
    assert_eq!(replay.installation_id, result.installation_id);
    assert_eq!(replay.generation_id, result.generation_id);
    assert_eq!(replay.idempotency_id, result.idempotency_id);
    let after_replay_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1",
    )
    .bind(result.idempotency_id)
    .fetch_one(&admin_pool)
    .await
    .expect("replayed installation event and outbox");
    let after_replay_bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_bindings WHERE installation_id = $1",
    )
    .bind(result.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("replayed installation binding count");
    assert_eq!(after_replay_counts, before_replay_counts);
    assert_eq!(after_replay_bindings, before_replay_bindings);
    sqlx::query(
        "UPDATE gateways
         SET lifecycle = 'enabled', active_revision_id = $2
         WHERE id = $1",
    )
    .bind(gateway_id)
    .bind(revision_id)
    .execute(&admin_pool)
    .await
    .expect("restore gateway serving authority");

    let managed_activation = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("general-managed-activate")
                    .expect("caller key"),
                installation_id: result.installation_id,
                expected_generation_id: Some(result.generation_id),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("assistant").expect("UI key"),
            },
        )
        .await
        .expect("managed/API activation");
    assert_ne!(managed_activation.generation_id, result.generation_id);
    let managed_activation_bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_bindings WHERE generation_id = $1",
    )
    .bind(managed_activation.generation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("managed activation bindings");
    assert_eq!(managed_activation_bindings, 2);
    let managed_rollback = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: UiInstallationCallerKey::parse("general-managed-rollback")
                    .expect("caller key"),
                installation_id: result.installation_id,
                expected_generation_id: Some(managed_activation.generation_id),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("assistant").expect("UI key"),
            },
        )
        .await
        .expect("managed/API rollback");
    assert_ne!(
        managed_rollback.generation_id,
        managed_activation.generation_id
    );
    let managed_rollback_bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_bindings WHERE generation_id = $1",
    )
    .bind(managed_rollback.generation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("managed rollback bindings");
    assert_eq!(managed_rollback_bindings, 2);

    let cross_project = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("general-cross-project")
                    .expect("caller key"),
                target: UiInstallationTarget::project(fixture.second_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("assistant").expect("UI key"),
                expected_organization_id: None,
                acknowledge_repository_git_access: false,
            },
        )
        .await
        .expect("same-organization cross-project managed/API installation");
    let cross_project_bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_bindings WHERE installation_id = $1",
    )
    .bind(cross_project.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("cross-project binding count");
    assert_eq!(cross_project_bindings, 2);

    scope_and_rejection::verify(&admin_pool, &worker_pool, &service).await;
}

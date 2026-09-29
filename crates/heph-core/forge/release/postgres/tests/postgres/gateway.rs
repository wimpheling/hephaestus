use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn seed_gateway_revision_variant(
    pool: &PgPool,
    base_revision_id: Uuid,
    gateway_id: Uuid,
    release_id: ReleaseId,
    release_agent_id: Uuid,
    exposure: &str,
    route_path: &str,
    methods: &[&str],
    enabled: bool,
    hash_byte: u8,
) -> Uuid {
    let revision_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         SELECT $1, gateway_id, project_id, repository_id, $2, $3,
                release_agent_key, handler_contract, $4, parameters,
                secret_slots, mailbox_slots, service_loopback_port,
                service_readiness_path, service_health_path, service_log_capture_mode,
                $5, created_by
         FROM gateway_revisions WHERE id = $6",
    )
    .bind(revision_id)
    .bind(release_id.as_uuid())
    .bind(release_agent_id)
    .bind(exposure)
    .bind(vec![hash_byte; 32])
    .bind(base_revision_id)
    .execute(pool)
    .await
    .expect("seed gateway revision variant");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods, enabled)
         VALUES ($1, $2, $3,
                 (SELECT project_id FROM gateway_revisions WHERE id = $4), $5, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(revision_id)
    .bind(gateway_id)
    .bind(base_revision_id)
    .bind(route_path)
    .bind(
        methods
            .iter()
            .map(|method| (*method).to_owned())
            .collect::<Vec<_>>(),
    )
    .bind(enabled)
    .execute(pool)
    .await
    .expect("seed gateway route variant");
    revision_id
}

pub(crate) async fn seed_same_org_target_project(
    pool: &PgPool,
    fixture: &Fixture,
    label: &str,
) -> ProjectId {
    let project = ProjectId::new();
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project.as_uuid())
        .bind(fixture.organization.as_uuid())
        .bind(format!("general-negative-{label}-{project}"))
        .execute(pool)
        .await
        .expect("seed same-organization target project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.as_uuid())
        .bind(fixture.actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed target project maintainer");
    project
}

pub(crate) async fn assert_installation_denied_without_receipt(
    service: &ReleaseService,
    pool: &PgPool,
    actor: UserId,
    target: ProjectId,
    release_id: ReleaseId,
    caller_key: String,
) {
    assert_installation_denied_without_receipt_as(
        service,
        pool,
        actor,
        target,
        release_id,
        caller_key,
        UiInstallationError::InvalidOrUnsupported,
    )
    .await;
}

pub(crate) async fn assert_installation_denied_without_receipt_as(
    service: &ReleaseService,
    pool: &PgPool,
    actor: UserId,
    target: ProjectId,
    release_id: ReleaseId,
    caller_key: String,
    expected: UiInstallationError,
) {
    let before: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM ui_installations WHERE project_id = $1),
             (SELECT count(*) FROM ui_installation_generations AS generation
              JOIN ui_installations AS installation ON installation.id = generation.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM ui_installation_bindings AS binding
              JOIN ui_installations AS installation ON installation.id = binding.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM ui_installation_commands AS command
              JOIN ui_installations AS installation ON installation.id = command.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM application_events WHERE aggregate_id = $1),
             (SELECT count(outbox.event_id) FROM application_events AS event
              LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
              WHERE event.aggregate_id = $1)",
    )
    .bind(target.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read denial baseline");
    let result = service
        .install_ui(
            &identity(actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse(&caller_key)
                    .expect("negative caller key"),
                target: UiInstallationTarget::project(target),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("assistant").expect("UI key"),
                expected_organization_id: None,
                acknowledge_repository_git_access: false,
            },
        )
        .await;
    assert!(
        matches!(&result, Err(actual) if *actual == expected),
        "{caller_key}: {result:?}"
    );
    let after: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM ui_installations WHERE project_id = $1),
             (SELECT count(*) FROM ui_installation_generations AS generation
              JOIN ui_installations AS installation ON installation.id = generation.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM ui_installation_bindings AS binding
              JOIN ui_installations AS installation ON installation.id = binding.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM ui_installation_commands AS command
              JOIN ui_installations AS installation ON installation.id = command.installation_id
              WHERE installation.project_id = $1),
             (SELECT count(*) FROM application_events WHERE aggregate_id = $1),
             (SELECT count(outbox.event_id) FROM application_events AS event
              LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
              WHERE event.aggregate_id = $1)",
    )
    .bind(target.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read denial result");
    assert_eq!(
        after, before,
        "{caller_key}: denied install changed durable state"
    );
}

/// Seeds only the durable source gateway state needed by the installation
/// resolver. No service instance is created: that remains runtime materializer
/// state, while this test proves immutable revision binding only.
// Keep the seed dimensions explicit so each gateway invariant is visible at
// the call site and the helper cannot silently choose a default contract.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn seed_active_ui_gateway(
    pool: &PgPool,
    fixture: &Fixture,
    release_id: ReleaseId,
    release_agent_id: Uuid,
    release_agent_key: &str,
    handler_contract: &str,
    exposure: &str,
    route_path: &str,
    methods: &[&str],
) -> (Uuid, Uuid) {
    let gateway_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let source_repository_id: Uuid =
        sqlx::query_scalar("SELECT repository_id FROM releases WHERE id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("read source repository for gateway");
    let source_project_id: Uuid =
        sqlx::query_scalar("SELECT project_id FROM repositories WHERE id = $1")
            .bind(source_repository_id)
            .fetch_one(pool)
            .await
            .expect("read source project for gateway");
    sqlx::query(
        "INSERT INTO gateways
         (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'ui-service', 'enabled', $4)",
    )
    .bind(gateway_id)
    .bind(source_project_id)
    .bind(source_repository_id)
    .bind(fixture.actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed source gateway");
    let service = handler_contract == "http.service.v1";
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '{}'::jsonb, '{}', '{}',
                 $10, $11, $12, 'disabled', $13, $14)",
    )
    .bind(revision_id)
    .bind(gateway_id)
    .bind(source_project_id)
    .bind(source_repository_id)
    .bind(release_id.as_uuid())
    .bind(release_agent_id)
    .bind(release_agent_key)
    .bind(handler_contract)
    .bind(exposure)
    .bind(service.then_some(8080_i32))
    .bind(service.then_some("/ready"))
    .bind(service.then_some("/health"))
    .bind(vec![7_u8; 32])
    .bind(fixture.actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed source gateway revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(revision_id)
    .bind(gateway_id)
    .bind(source_project_id)
    .bind(route_path)
    .bind(
        methods
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    )
    .execute(pool)
    .await
    .expect("seed source gateway route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate source gateway revision");
    (gateway_id, revision_id)
}

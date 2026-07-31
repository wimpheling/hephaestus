//! Structural regression tests for the durable product-event boundary.

const MIGRATION: &str = include_str!("../../../migrations/0010_durable_application_events.sql");

#[test]
fn every_authoritative_product_state_path_has_a_capture_trigger() {
    let sources = [
        "users",
        "external_identities",
        "user_profiles",
        "organizations",
        "organization_members",
        "organization_secret_managers",
        "projects",
        "project_maintainers",
        "project_secret_roles",
        "repositories",
        "repository_managers",
        "repository_secret_roles",
        "agent_families",
        "git_receives",
        "git_refs",
        "git_ref_updates",
        "agent_config_revisions",
        "build_requests",
        "build_request_sources",
        "build_executions",
        "releases",
        "release_agents",
        "release_artifacts",
        "agent_instances",
        "agent_instance_revisions",
        "agent_attachments",
        "agent_updates",
        "agent_instance_state_volumes",
        "deferred_agent_triggers",
        "agent_instance_events",
        "agent_instance_volume_leases",
        "runs",
        "run_requests",
        "run_results",
        "run_workspaces",
        "run_instance_provenance",
        "run_secret_provenance",
        "result_artifacts",
        "review_proposals",
        "control_requests",
        "secrets",
        "secret_versions",
        "secret_grants",
        "secret_imports",
        "agent_secret_bindings",
        "secret_leases",
        "secret_runtime_sessions",
        "secret_runtime_mounts",
    ];
    for table in sources {
        assert!(
            MIGRATION.contains(&format!(" ON {table}\n")),
            "missing application-event trigger for {table}"
        );
    }
}

#[test]
fn product_outbox_is_trigger_only_and_dead_letters_do_not_pin_retention() {
    assert!(MIGRATION.contains("AFTER INSERT ON application_events"));
    assert!(MIGRATION.contains("INSERT INTO product_event_outbox (event_id) VALUES (NEW.id)"));
    assert!(!MIGRATION.contains("GRANT INSERT ON product_event_outbox"));
    assert!(MIGRATION.contains("AND outbox.dead_lettered_at IS NULL"));
    assert!(MIGRATION.contains("CREATE TABLE product_event_dead_letters"));
    assert!(MIGRATION.contains("CHECK (num_nonnulls(published_at, dead_lettered_at) <= 1)"));
}

#[test]
fn persisted_projection_domain_is_finite_and_non_disclosing() {
    for aggregate in [
        "identity_profile",
        "identity_organizations",
        "organization",
        "project",
        "repository",
        "repository_ref",
        "build",
        "release",
        "agent_instance",
        "run",
        "review",
        "secret_metadata",
        "secret_grant",
        "secret_import",
        "agent_secret_binding",
        "artifact",
    ] {
        assert!(MIGRATION.contains(&format!("'{aggregate}'")));
    }
    assert!(MIGRATION.contains("unknown application event state %"));
    assert!(MIGRATION.contains("CASE aggregate_type"));
    assert!(!MIGRATION.contains("safe_ref text"));
    assert!(!MIGRATION.contains("old_commit text"));
    assert!(!MIGRATION.contains("new_commit text"));
}

#[test]
fn secret_authority_changes_share_occurrence_across_owner_and_project() {
    let parented = MIGRATION
        .split("CREATE FUNCTION capture_parented_application_event")
        .nth(1)
        .expect("parented capture function");
    for table in ["secret_grants", "secret_imports"] {
        let route = parented
            .split(&format!("WHEN '{table}' THEN"))
            .nth(1)
            .expect("secret route");
        let route = route.split("WHEN '").next().expect("bounded route");
        assert!(route.contains("v_occurrence, 'organization'"));
        assert!(route.contains("v_occurrence, 'project'"));
        assert!(!route.contains("v_occurrence, 'repository'"));
    }
}

#[test]
fn run_changes_share_one_occurrence_across_run_instance_and_project_scopes() {
    let parented = MIGRATION
        .split("CREATE FUNCTION capture_parented_application_event")
        .nth(1)
        .expect("parented capture function");
    let route = parented
        .split("WHEN 'runs' THEN")
        .nth(1)
        .expect("run route")
        .split("WHEN 'result_artifacts' THEN")
        .next()
        .expect("bounded run route");

    assert_eq!(route.matches("v_occurrence").count(), 3);
    assert!(route.contains("v_occurrence, 'run', v_id, 'run', v_id"));
    assert!(route.contains("v_occurrence, 'agent_instance', v_parent, 'run', v_id"));
    assert!(route.contains("v_occurrence, 'project', v_project, 'run', v_id"));
    assert!(MIGRATION.contains("CREATE TRIGGER runs_application_event"));
    assert!(!MIGRATION.contains("CREATE TRIGGER runs_run_application_event"));
}

#[test]
fn capture_uses_stable_transaction_occurrence_for_idempotent_receipts() {
    let occurrence_expression =
        "NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid";
    assert_eq!(MIGRATION.matches(occurrence_expression).count(), 2);
    assert!(!MIGRATION.contains("gen_random_uuid(), TG_ARGV[2]"));
}

#[test]
fn internal_outbox_cannot_be_misclassified_as_product_events() {
    assert!(MIGRATION.contains("'internal_command'"));
    assert!(MIGRATION.contains("'internal_signal'"));
    assert!(!MIGRATION.contains("message_class IN ('product_event'"));
    assert!(MIGRATION.contains("CHECK (subject = 'hephaestus.product.event.v1')"));
    let retirement = MIGRATION
        .split("UPDATE outbox")
        .nth(1)
        .expect("bounded legacy retirement")
        .split(");")
        .next()
        .expect("retirement statement");
    assert!(retirement.contains("WHERE published_at IS NULL AND subject IN ("));
    assert!(retirement.contains("'hephaestus.release.published.v1'"));
    assert!(retirement.contains("'hephaestus.git.receive.accepted'"));
    assert!(retirement.contains("'hephaestus.git.agent_config.invalid'"));
    assert!(retirement.contains("'heph.run.event.lifecycle.v1'"));
    assert!(retirement.contains("'hephaestus.secret.reconcile_revocation.v1'"));
    for actionable in [
        "hephaestus.build.requested.v1",
        "hephaestus.instance.run.requested.v1",
        "heph.run.command.start.v1",
        "heph.run.command.cancel.v1",
        "hephaestus.control.execute",
    ] {
        assert!(MIGRATION.contains(&format!("'{actionable}'")));
        assert!(!retirement.contains(actionable));
    }
}

#[tokio::test]
#[serial_test::serial]
async fn postgres_events_are_atomic_ordered_versioned_and_multi_scope() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect event PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply event migrations");

    let actor = uuid::Uuid::new_v4();
    let organization = uuid::Uuid::new_v4();
    let project = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Event Actor')")
        .bind(actor)
        .execute(&pool)
        .await
        .expect("seed actor");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("event-{organization}"))
        .execute(&pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
           VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("seed membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("event-{project}"))
        .execute(&pool)
        .await
        .expect("seed project");

    let baseline: i64 = sqlx::query_scalar(
        "SELECT committed_cursor FROM application_event_scopes
           WHERE scope_kind = 'project' AND scope_id = $1",
    )
    .bind(project)
    .fetch_one(&pool)
    .await
    .expect("project cursor");
    let rolled_back_request = uuid::Uuid::new_v4();
    let mut transaction = pool.begin().await.expect("rollback transaction");
    set_actor(&mut transaction, actor, rolled_back_request).await;
    sqlx::query("UPDATE projects SET settings = '{\"rolled_back\":true}' WHERE id = $1")
        .bind(project)
        .execute(&mut *transaction)
        .await
        .expect("update rolled-back project");
    transaction
        .rollback()
        .await
        .expect("rollback state and event");
    let after_rollback: i64 = sqlx::query_scalar(
        "SELECT committed_cursor FROM application_event_scopes
           WHERE scope_kind = 'project' AND scope_id = $1",
    )
    .bind(project)
    .fetch_one(&pool)
    .await
    .expect("cursor after rollback");
    assert_eq!(after_rollback, baseline);
    let leaked: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM application_events WHERE request_id = $1)")
            .bind(rolled_back_request)
            .fetch_one(&pool)
            .await
            .expect("rolled-back event query");
    assert!(!leaked);

    let first_request = uuid::Uuid::new_v4();
    let second_request = uuid::Uuid::new_v4();
    let first = update_project(&pool, actor, project, first_request, "first");
    let second = update_project(&pool, actor, project, second_request, "second");
    let (first_result, second_result) = tokio::join!(first, second);
    first_result.expect("first concurrent update");
    second_result.expect("second concurrent update");
    let ordered: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT cursor, aggregate_version FROM application_events
           WHERE scope_kind = 'project' AND scope_id = $1
             AND request_id IN ($2, $3)
           ORDER BY cursor",
    )
    .bind(project)
    .bind(first_request)
    .bind(second_request)
    .fetch_all(&pool)
    .await
    .expect("ordered concurrent events");
    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[1].0, ordered[0].0 + 1);
    assert_eq!(ordered[1].1, ordered[0].1 + 1);

    let secret = uuid::Uuid::new_v4();
    let secret_request = uuid::Uuid::new_v4();
    let mut transaction = pool.begin().await.expect("secret transaction");
    set_actor(&mut transaction, actor, secret_request).await;
    sqlx::query(
        "INSERT INTO secrets (
              id, owner_organization_id, project_id, name, status,
              allowed_delivery_modes, created_by
           ) VALUES ($1, $2, $3, $4, 'active', ARRAY['brokered'], $5)",
    )
    .bind(secret)
    .bind(organization)
    .bind(project)
    .bind(format!("secret-{}", &secret.to_string()[..8]))
    .bind(actor)
    .execute(&mut *transaction)
    .await
    .expect("create project secret");
    transaction.commit().await.expect("commit secret");
    let occurrences: Vec<(String, uuid::Uuid)> = sqlx::query_as(
        "SELECT scope_kind, occurrence_id FROM application_events
           WHERE request_id = $1 AND aggregate_type = 'secret_metadata'
           ORDER BY scope_kind",
    )
    .bind(secret_request)
    .fetch_all(&pool)
    .await
    .expect("secret scope events");
    assert_eq!(occurrences.len(), 2);
    assert_eq!(occurrences[0].1, occurrences[1].1);
    assert_eq!(occurrences[0].1, secret_request);
    assert_eq!(occurrences[0].0, "organization");
    assert_eq!(occurrences[1].0, "project");
}

async fn set_actor(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: uuid::Uuid,
    request: uuid::Uuid,
) {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                  set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $2, true),
                  set_config('hephaestus.occurrence_id', $2, true)",
    )
    .bind(actor.to_string())
    .bind(request.to_string())
    .execute(&mut **transaction)
    .await
    .expect("set event actor");
}

async fn update_project(
    pool: &sqlx::PgPool,
    actor: uuid::Uuid,
    project: uuid::Uuid,
    request: uuid::Uuid,
    key: &str,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    set_actor(&mut transaction, actor, request).await;
    sqlx::query(
        "UPDATE projects
           SET settings = settings || jsonb_build_object($2::text, true)
           WHERE id = $1",
    )
    .bind(project)
    .bind(key)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
}

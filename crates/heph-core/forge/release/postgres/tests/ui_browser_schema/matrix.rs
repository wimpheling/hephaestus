use super::*;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_browser_schema_matrix_enforces_bindings_lifecycle_timing_and_roles() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser schema: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0097");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker");
    assert!(max_migration >= EXPECTED_MIGRATION);

    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    assert_role(&worker, "hephaestus_worker", false, true).await;
    assert_role(&app, "hephaestus_app", false, false).await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;

    let partial_context = sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, installation_id, occurred_at)
         VALUES ($1, $2, 'handoff_issue', 'denied', 'not_attempted',
                 'unauthorized', $3, $4, statement_timestamp())",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.installation)
    .execute(&worker)
    .await;
    let partial_error = take_query_error(
        partial_context,
        "partial audit context unexpectedly succeeded",
    );
    assert_database_code(&partial_error, "23514", "partial verified target tuple");

    let audit_child = insert_audit_child(&worker, &fixture).await;
    let child_actor_mismatch = sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, organization_id, installation_id, generation_id,
             child_session_id, occurred_at)
         VALUES ($1, $2, 'handoff_exchange', 'allowed', 'succeeded', 'none',
                 $3, $4, $5, $6, $7, statement_timestamp())",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fixture.outsider)
    .bind(fixture.organization)
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(audit_child)
    .execute(&worker)
    .await;
    let child_error = take_query_error(
        child_actor_mismatch,
        "child actor mismatch unexpectedly succeeded",
    );
    assert_database_code(&child_error, "23000", "child actor context");

    let valid_gateway_request = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, organization_id, installation_id, generation_id,
             gateway_id, gateway_revision_id, occurred_at)
         VALUES ($1, $2, 'managed', 'allowed', 'succeeded', 'none',
                 $3, $4, $5, $6, $7, $8, statement_timestamp())",
    )
    .bind(Uuid::new_v4())
    .bind(valid_gateway_request)
    .bind(fixture.actor)
    .bind(fixture.organization)
    .bind(fixture.managed_installation)
    .bind(fixture.managed_generation)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .execute(&worker)
    .await
    .expect("generation gateway binding is accepted");
    let gateway_mismatch = sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, organization_id, installation_id, generation_id,
             gateway_id, gateway_revision_id, occurred_at)
         VALUES ($1, $2, 'managed', 'allowed', 'succeeded', 'none',
                 $3, $4, $5, $6, $7, $8, statement_timestamp())",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.organization)
    .bind(fixture.other_installation)
    .bind(fixture.other_generation)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .execute(&worker)
    .await;
    let gateway_error = take_query_error(
        gateway_mismatch,
        "unbound gateway revision unexpectedly succeeded",
    );
    assert_database_code(&gateway_error, "23000", "historical gateway binding");

    assert_digest_constraints(&worker, &fixture).await;
    assert_wrong_actor_binding(&worker, &fixture).await;
    assert_wrong_organization_binding(&worker, &fixture).await;
    assert_wrong_installation_generation_binding(&worker, &fixture).await;
    assert_wrong_child_binding(&worker, &fixture).await;
    assert_initial_consumption_is_rejected(&worker, &fixture).await;
    assert_child_issue_interval_and_parent_cap(&worker, &fixture).await;
    assert_child_only_commit_rolls_back(&worker, &fixture).await;
    assert_consume_only_is_rejected(&worker, &fixture).await;
    assert_commit_and_consume_is_one_time(&worker, &fixture).await;
    assert_expiry_and_twelve_hour_cap(&worker, &fixture).await;
    assert_application_role_is_denied(&app, &fixture).await;

    println!(
        "REAL_UI_BROWSER_SCHEMA=1 migration={max_migration} digest=1 binding=1 initial_consumed=1 child_interval=1 transaction=1 one_time=1 expiry=1 role_denial=1"
    );
}

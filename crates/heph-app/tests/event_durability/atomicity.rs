use super::support::{set_actor, update_project};

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
    let injected_failure = sqlx::query("SELECT 1 / 0").execute(&mut *transaction).await;
    assert!(
        injected_failure.is_err(),
        "failure injection unexpectedly succeeded"
    );
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
    let leaked_outbox: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1
             FROM product_event_outbox outbox
             JOIN application_events event ON event.id = outbox.event_id
             WHERE event.request_id = $1
         )",
    )
    .bind(rolled_back_request)
    .fetch_one(&pool)
    .await
    .expect("rolled-back outbox query");
    assert!(!leaked_outbox);

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
    let first_pending: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM product_event_outbox outbox
         JOIN application_events event ON event.id = outbox.event_id
         WHERE event.request_id = $1 AND outbox.published_at IS NULL
           AND outbox.dead_lettered_at IS NULL",
    )
    .bind(first_request)
    .fetch_one(&pool)
    .await
    .expect("first pending product event");
    // A project mutation intentionally captures both its project and
    // organization scopes; each committed scope has one pending outbox row.
    assert_eq!(first_pending, 2);

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

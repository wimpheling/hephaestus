use super::support::{set_actor, transition_gateway};

#[tokio::test]
#[serial_test::serial]
async fn postgres_gateway_lifecycle_event_is_atomic_receipted_and_compare_and_swap_safe() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect gateway event PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway event migrations");

    let actor = uuid::Uuid::new_v4();
    let organization = uuid::Uuid::new_v4();
    let project = uuid::Uuid::new_v4();
    let repository = uuid::Uuid::new_v4();
    let gateway = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway Event Actor')")
        .bind(actor)
        .execute(&pool)
        .await
        .expect("seed gateway actor");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("gateway-event-{organization}"))
        .execute(&pool)
        .await
        .expect("seed gateway organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("seed gateway membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("gateway-event-{project}"))
        .execute(&pool)
        .await
        .expect("seed gateway project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("gateway-event-{}", &repository.to_string()[..8]))
        .execute(&pool)
        .await
        .expect("seed gateway repository");

    let create_request = uuid::Uuid::new_v4();
    let mut create = pool.begin().await.expect("begin gateway insert");
    set_actor(&mut create, actor, create_request).await;
    sqlx::query(
        "INSERT INTO gateways (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, 'enabled', $5)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!("gateway-{}", &gateway.to_string()[..8]))
    .bind(actor)
    .execute(&mut *create)
    .await
    .expect("insert gateway");
    create.commit().await.expect("commit gateway insert");

    let create_event: (String, String, String, uuid::Uuid) = sqlx::query_as(
        "SELECT aggregate_type, event_type, change_kind, request_id
         FROM application_events WHERE aggregate_id = $1 ORDER BY cursor",
    )
    .bind(gateway)
    .fetch_one(&pool)
    .await
    .expect("gateway create event");
    assert_eq!(create_event.0, "gateway");
    assert_eq!(create_event.1, "gateway.changed");
    assert_eq!(create_event.2, "created");
    assert_eq!(create_event.3, create_request);

    let first_request = uuid::Uuid::new_v4();
    let second_request = uuid::Uuid::new_v4();
    let first = transition_gateway(&pool, actor, gateway, first_request);
    let second = transition_gateway(&pool, actor, gateway, second_request);
    let (first, second) = tokio::join!(first, second);
    assert_ne!(
        first.expect("first transition"),
        second.expect("second transition")
    );

    let lifecycle_events: Vec<(String, String, uuid::Uuid)> = sqlx::query_as(
        "SELECT change_kind, safe_state, request_id
         FROM application_events WHERE aggregate_id = $1 AND event_type = 'gateway.changed'
         ORDER BY cursor",
    )
    .bind(gateway)
    .fetch_all(&pool)
    .await
    .expect("gateway events");
    assert_eq!(lifecycle_events.len(), 2);
    assert_eq!(lifecycle_events[1].0, "state_changed");
    assert_eq!(lifecycle_events[1].1, "paused");
    assert!(
        matches!(lifecycle_events[1].2, request if request == first_request || request == second_request)
    );
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM product_event_outbox outbox
         JOIN application_events event ON event.id = outbox.event_id
         WHERE event.aggregate_id = $1",
    )
    .bind(gateway)
    .fetch_one(&pool)
    .await
    .expect("gateway product outbox rows");
    assert_eq!(queued, 2);
}

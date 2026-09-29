use super::support::set_actor;
use event_application::ProductEventOutbox;
use event_postgres::PostgresProductEventOutbox;

#[tokio::test]
#[serial_test::serial]
async fn open_transaction_is_invisible_to_event_publisher_until_commit() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect event visibility PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply event visibility migrations");

    let actor = uuid::Uuid::new_v4();
    let organization = uuid::Uuid::new_v4();
    let project = uuid::Uuid::new_v4();
    let request = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Event Visibility Actor')")
        .bind(actor)
        .execute(&pool)
        .await
        .expect("seed visibility actor");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("event-visibility-{organization}"))
        .execute(&pool)
        .await
        .expect("seed visibility organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
           VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("seed visibility membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("event-visibility-{project}"))
        .execute(&pool)
        .await
        .expect("seed visibility project");

    let mut transaction = pool.begin().await.expect("begin visibility transaction");
    set_actor(&mut transaction, actor, request).await;
    sqlx::query("UPDATE users SET display_name = 'Event Visibility Updated' WHERE id = $1")
        .bind(actor)
        .execute(&mut *transaction)
        .await
        .expect("update uncommitted project");

    let publisher = PostgresProductEventOutbox::new(pool.clone());
    let before_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE request_id = $1")
            .bind(request)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted events");
    assert_eq!(before_events, 0);
    let before_pending = publisher
        .pending(10_000)
        .await
        .expect("read pending events before commit");
    assert!(
        !before_pending
            .iter()
            .any(|event| event.request_id == Some(request))
    );

    transaction
        .commit()
        .await
        .expect("commit visibility transaction");
    let after_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE request_id = $1")
            .bind(request)
            .fetch_one(&pool)
            .await
            .expect("read committed event");
    assert_eq!(after_events, 1);
    let after_pending = publisher
        .pending(10_000)
        .await
        .expect("read pending event after commit");
    assert_eq!(
        after_pending
            .iter()
            .filter(|event| event.request_id == Some(request))
            .count(),
        1
    );
}

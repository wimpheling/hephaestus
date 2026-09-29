use identity_domain::RequestId;
use mailbox_postgres::PostgresMailboxRepository;
use runtime_types::AgentInstanceId;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;

use super::support::seed_instance;

#[tokio::test]
#[serial]
async fn instance_mailbox_allocation_is_authorized_idempotent_and_receipted() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("SKIP mailbox allocation integration: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply mailbox allocation migration");
    let fixture = seed_instance(&pool).await;
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(fixture.project)
        .bind(fixture.owner.user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("project manager");
    let store = PostgresMailboxRepository::new(pool.clone());
    let first = store
        .allocate(&fixture.owner, fixture.instance)
        .await
        .expect("project and instance manager allocates mailbox");
    let replay = store
        .allocate(&fixture.owner, fixture.instance)
        .await
        .expect("same request retries idempotently");
    assert_eq!(first, replay);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mailboxes WHERE instance_id = $1",)
            .bind(fixture.instance.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count allocated mailbox"),
        1
    );
    let fresh_operation = fixture.owner.clone().with_idempotency_id(RequestId::new());
    assert_eq!(
        store
            .allocate(&fresh_operation, fixture.instance)
            .await
            .expect("fresh operation reuses the instance mailbox"),
        first
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_events
             WHERE occurrence_id = $1 AND scope_kind = 'agent_instance'
               AND scope_id = $2 AND aggregate_type = 'agent_instance'",
        )
        .bind(fixture.owner.idempotency_id.as_uuid())
        .bind(fixture.instance.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load allocation receipt event"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_events
             WHERE occurrence_id = $1 AND scope_kind = 'agent_instance'
               AND scope_id = $2 AND aggregate_type = 'agent_instance'",
        )
        .bind(fresh_operation.idempotency_id.as_uuid())
        .bind(fixture.instance.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load fresh allocation receipt event"),
        1
    );
    let other_instance = AgentInstanceId::new();
    sqlx::query(
        "INSERT INTO agent_instances (id, project_id, family_id, name, state)
         SELECT $1, project_id, family_id, $3, 'active'
         FROM agent_instances WHERE id = $2",
    )
    .bind(other_instance.as_uuid())
    .bind(fixture.instance.as_uuid())
    .bind(format!("mailbox-{other_instance}"))
    .execute(&pool)
    .await
    .expect("second managed instance");
    assert!(matches!(
        store.allocate(&fixture.owner, other_instance).await,
        Err(mailbox_postgres::MailboxPersistenceError::IdempotencyConflict)
    ));
    assert!(matches!(
        store.allocate(&fixture.outsider, fixture.instance).await,
        Err(mailbox_postgres::MailboxPersistenceError::Unavailable)
    ));
    sqlx::query("UPDATE agent_instances SET state = 'removed', removed_at = now() WHERE id = $1")
        .bind(fixture.instance.as_uuid())
        .execute(&pool)
        .await
        .expect("retire instance");
    let retired_retry = fixture.owner.clone().with_idempotency_id(RequestId::new());
    assert!(matches!(
        store.allocate(&retired_retry, fixture.instance).await,
        Err(mailbox_postgres::MailboxPersistenceError::Unavailable)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM mailboxes WHERE instance_id = $1")
            .bind(fixture.instance.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("read retired mailbox state"),
        "active"
    );
}

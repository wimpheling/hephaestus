//! `PostgreSQL` mutation receipt adapter contract tests.

use event_application::{MutationReceiptError, MutationReceiptReader};
use event_postgres::PostgresMutationReceiptReader;
use identity_domain::{RequestId, UserId};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn loads_only_the_latest_exact_committed_mutation_event() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect event PostgreSQL adapter test");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply event PostgreSQL adapter migrations");

    let actor_id = UserId::new();
    let occurrence_id = RequestId::new();
    let organization_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Receipt Actor')")
        .bind(actor_id.as_uuid())
        .execute(&pool)
        .await
        .expect("seed receipt actor");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Receipt Organization')")
        .bind(organization_id)
        .execute(&pool)
        .await
        .expect("seed receipt organization");

    for name in ["Receipt Organization One", "Receipt Organization Two"] {
        let mut transaction = pool.begin().await.expect("begin receipt mutation");
        sqlx::query(
            "SELECT set_config('hephaestus.actor_id', $1, true),
                    set_config('hephaestus.subject_type', 'user', true),
                    set_config('hephaestus.request_id', $2, true),
                    set_config('hephaestus.occurrence_id', $3, true)",
        )
        .bind(actor_id.to_string())
        .bind(RequestId::new().to_string())
        .bind(occurrence_id.to_string())
        .execute(&mut *transaction)
        .await
        .expect("install receipt mutation identity");
        sqlx::query("UPDATE organizations SET name = $2 WHERE id = $1")
            .bind(organization_id)
            .bind(name)
            .execute(&mut *transaction)
            .await
            .expect("mutate receipt organization");
        transaction.commit().await.expect("commit receipt mutation");
    }

    let reader = PostgresMutationReceiptReader::new(pool);
    let receipt = reader
        .load(occurrence_id, actor_id, "organization", "organization")
        .await
        .expect("load committed mutation receipt");
    assert_eq!(receipt.scope_kind, "organization");
    assert_eq!(receipt.scope_id, organization_id);
    assert_eq!(receipt.cursor, 3);
    assert_eq!(receipt.aggregate_version, 3);

    for result in [
        reader
            .load(occurrence_id, UserId::new(), "organization", "organization")
            .await,
        reader
            .load(occurrence_id, actor_id, "project", "organization")
            .await,
        reader
            .load(occurrence_id, actor_id, "organization", "project")
            .await,
    ] {
        assert!(matches!(result, Err(MutationReceiptError::Missing)));
    }
}

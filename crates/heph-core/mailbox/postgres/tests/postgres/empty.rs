use mailbox_domain::MailboxId;
use mailbox_postgres::PostgresMailboxRepository;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;

use super::support::{event, seed_instance};

#[tokio::test]
#[serial]
async fn empty_opaque_body_is_accepted_with_exact_zero_length_evidence() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
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
        .expect("apply mailbox migrations");
    let fixture = seed_instance(&pool).await;
    let store = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    store
        .ensure_mailbox(fixture.project, mailbox_id, fixture.instance)
        .await
        .expect("create instance-owned mailbox");

    let accepted = store
        .accept(
            fixture.project,
            &event(mailbox_id, fixture.instance, b""),
            b"",
            0,
        )
        .await
        .expect("accept an empty opaque body");
    let (encoded_length, decoded_length, stored_length): (i32, i32, i32) = sqlx::query_as(
        "SELECT encoded_length, decoded_length, octet_length(encoded_body)
         FROM mailbox_payloads
         WHERE id = (SELECT body_id FROM mailbox_events WHERE id = $1)",
    )
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load exact empty payload evidence");
    assert_eq!((encoded_length, decoded_length, stored_length), (0, 0, 0));
}

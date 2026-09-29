use sqlx::Postgres;

async fn acknowledge(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
) {
    sqlx::query(
        "UPDATE product_event_outbox
         SET published_at = now()
         WHERE event_id = $1",
    )
    .execute(&mut **transaction)
    .await
    .unwrap();
}

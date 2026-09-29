use sqlx::Postgres;

async fn record_change(transaction: &mut sqlx::Transaction<'_, Postgres>) {
    sqlx::query(
        "SELECT event_id FROM append_application_event(
             $1, 'project', $2, 'project', $2,
             'project.changed', 'updated', 'active', NULL, NULL
         )",
    )
    .fetch_one(&mut **transaction)
    .await
    .unwrap();
}

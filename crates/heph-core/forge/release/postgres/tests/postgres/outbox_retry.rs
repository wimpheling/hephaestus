use super::*;
use event_postgres::ReleaseOutboxPublisher;

#[tokio::test]
#[serial]
async fn release_outbox_retry_is_deduplicated_by_jetstream() {
    let (Ok(nats_url), Some(pool)) = (std::env::var("HEPHAESTUS_NATS_TEST_URL"), pool().await)
    else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    sqlx::query(
        "UPDATE outbox SET published_at = now()
         WHERE aggregate_type = 'release' AND published_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("isolate release outbox fixture");
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES
         ($1, 'release', $2, 'hephaestus.instance.run.requested.v1',
          'instance.run.requested.v1', '{}', now()),
         ($3, 'release', $4, 'hephaestus.run.start',
          'run.start.v1', '{}', now())",
    )
    .bind(first_id)
    .bind(Uuid::new_v4())
    .bind(second_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("insert release outbox fixture");

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let stream_name = format!("HEPH_RELEASE_TEST_{}", first_id.simple());
    let mut stream = context
        .create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![String::from("hephaestus.>")],
            duplicate_window: Duration::from_secs(60),
            ..Default::default()
        })
        .await
        .expect("isolated release stream");
    let publisher = ReleaseOutboxPublisher::new(context.clone(), pool.clone());
    assert_eq!(
        publisher
            .publish_pending(10)
            .await
            .expect("first publication"),
        2
    );
    assert_eq!(stream.info().await.expect("stream state").state.messages, 2);
    sqlx::query("UPDATE outbox SET published_at = NULL WHERE id IN ($1, $2)")
        .bind(first_id)
        .bind(second_id)
        .execute(&pool)
        .await
        .expect("simulate acknowledgement loss");
    assert_eq!(
        publisher
            .publish_pending(10)
            .await
            .expect("retry publication"),
        2
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("deduplicated state")
            .state
            .messages,
        2
    );

    context
        .delete_stream(&stream_name)
        .await
        .expect("delete isolated stream");
    sqlx::query("DELETE FROM outbox WHERE id IN ($1, $2)")
        .bind(first_id)
        .bind(second_id)
        .execute(&pool)
        .await
        .expect("clean release outbox fixture");
}

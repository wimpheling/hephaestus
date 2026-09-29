use super::*;

#[tokio::test]
#[serial]
async fn forge_outbox_retry_is_deduplicated_by_jetstream() {
    let (Ok(nats_url), Some((pool, service, repository, temporary))) =
        (std::env::var("HEPHAESTUS_NATS_TEST_URL"), fixture().await)
    else {
        return;
    };
    // Opt-in test binaries may share one explicitly configured database.
    sqlx::query(
        "UPDATE outbox SET published_at = now()
         WHERE aggregate_type = 'forge' AND published_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("isolate forge outbox fixture");
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (_, update) = commit_and_update(&temporary, &repository, &config).await;
    service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("accepted receive");
    let command_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM outbox
         WHERE published_at IS NULL
           AND subject IN (
               'hephaestus.build.requested.v1',
               'hephaestus.instance.run.requested.v1',
               'hephaestus.run.start'
           )
         ORDER BY occurred_at, id",
    )
    .fetch_all(&pool)
    .await
    .expect("exact receive command identities");
    assert!(!command_ids.is_empty());

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let stream_name = format!("HEPH_PHASE2_TEST_{}", repository.id.as_uuid().simple());
    let mut stream = context
        .create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![
                String::from(BUILD_REQUESTED_SUBJECT),
                String::from(INSTANCE_RUN_REQUESTED_SUBJECT),
                String::from(RUN_START_SUBJECT),
            ],
            duplicate_window: Duration::from_secs(60),
            ..Default::default()
        })
        .await
        .expect("isolated forge stream");
    let publisher = ForgeNatsOutboxPublisher::new(context.clone());
    assert_eq!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("first publication"),
        command_ids.len()
    );
    assert_eq!(
        stream.info().await.expect("stream state").state.messages,
        u64::try_from(command_ids.len()).expect("bounded command count")
    );
    sqlx::query("UPDATE outbox SET published_at = NULL WHERE id = ANY($1)")
        .bind(&command_ids)
        .execute(&pool)
        .await
        .expect("simulate acknowledgement loss");
    assert_eq!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("retry publication"),
        command_ids.len()
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("deduplicated state")
            .state
            .messages,
        u64::try_from(command_ids.len()).expect("bounded command count")
    );

    context
        .delete_stream(&stream_name)
        .await
        .expect("delete isolated stream");
    cleanup(&pool, repository).await;
}
